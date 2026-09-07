"""REL session ownership, pacing, readiness, and source recovery."""

from __future__ import annotations

import logging
import time
from datetime import UTC, datetime
from pathlib import Path

from .api import PageOperation, RelRpcError, RelTransportError
from .errors import CrawlConfigurationError, CrawlRecoveryError
from .html import canonicalize_url

SESSION_RESTART_RPC_IDS = {
    "AGENT_UNHEALTHY",
    "BROWSER_UNAVAILABLE",
    "SESSION_NOT_FOUND",
}
_LOGGER = logging.getLogger("rel_crawler.crawler")


class SessionMixin:
    """Private engine mixin for browser-session lifecycle and recovery."""

    def _back_to_source(self, session_id: str, recovery_output: Path) -> None:
        try:
            _LOGGER.info("navigating Back to source in session %s", session_id)
            self._before_browser_action()
            navigation = self.client.back(
                session_id=session_id, timeout=self.timeout, wait=self.wait
            )
            _LOGGER.info("Back navigation returned url=%s", navigation.url)
            if canonicalize_url(navigation.url) == self._source_url:
                if self.source_ready_selector is not None:
                    operation = self._wait_for_ready(
                        session_id,
                        self.source_ready_selector,
                        recovery_output,
                    )
                    if canonicalize_url(operation.url) != self._source_url:
                        raise CrawlRecoveryError(
                            f"source readiness check ended at {operation.url!r}"
                        )
                _LOGGER.info("source restored through browser history")
                return
            _LOGGER.warning(
                "Back reached unexpected url=%s; reloading source directly",
                navigation.url,
            )
        except Exception as error:  # noqa: BLE001 - any Back failure needs recovery
            _LOGGER.warning(
                "Back/source readiness failed; reloading source directly: %s: %s",
                type(error).__name__,
                error,
            )
        self._navigate_to_source(session_id, recovery_output)

    def _navigate_to_source(self, session_id: str, recovery_output: Path) -> None:
        source_uri = canonicalize_url(self.definition.start_url)
        _LOGGER.info(
            "reloading source directly at %s%s",
            self.definition.start_url,
            (f" (URI {source_uri})" if self.definition.start_url != source_uri else ""),
        )
        self._before_browser_action()
        operation = self.client.navigate(
            url=source_uri,
            session_id=session_id,
            output=recovery_output,
            timeout=self.timeout,
            wait=self.wait,
        )
        _LOGGER.info(
            "source recovery navigation returned url=%s status=%s",
            operation.url,
            operation.target_http_status,
        )
        if self.source_ready_selector is not None:
            operation = self._wait_for_ready(
                session_id,
                self.source_ready_selector,
                recovery_output,
            )
        self._source_url = canonicalize_url(operation.url)
        self._state["source_url"] = operation.url
        self._save_state()
        _LOGGER.info("source recovery is ready at %s", operation.url)
        if self._load_more_depth:
            self._replay_load_more(
                session_id,
                recovery_output,
                depth=self._load_more_depth,
            )

    def _ensure_session(self) -> str:
        state_session = self._state.get("session_id")
        if self.requested_session_id is not None:
            _LOGGER.info(
                "checking caller-owned REL session %s",
                self.requested_session_id,
            )
            if state_session is not None and state_session != self.requested_session_id:
                raise CrawlConfigurationError(
                    f"checkpoint belongs to {state_session}, "
                    f"not {self.requested_session_id}"
                )
            if self.client.get_session(self.requested_session_id) is None:
                raise CrawlConfigurationError(
                    f"REL session {self.requested_session_id} does not exist"
                )
            self._state["session_id"] = self.requested_session_id
            self._state["managed_session"] = False
            self._state["session_generation"] = 0
            self._state["pending_session_restart"] = None
            self._save_state()
            return self.requested_session_id

        if isinstance(state_session, str):
            _LOGGER.info("checking checkpoint REL session %s", state_session)
            session = self.client.get_session(state_session)
            if session is not None:
                live_profile = session.get("profile")
                profile_changed = (
                    self._state.get("managed_session")
                    and isinstance(live_profile, str)
                    and self.profile is not None
                    and live_profile.casefold() != self.profile.casefold()
                )
                if not profile_changed:
                    _LOGGER.info("reusing checkpoint REL session %s", state_session)
                    return state_session
                _LOGGER.info(
                    "checkpoint session profile changed from %r to %r",
                    live_profile,
                    self.profile,
                )
                return self._restart_managed_session(
                    state_session,
                    reason=(
                        f"profile changed from {live_profile!r} to {self.profile!r}"
                    ),
                )
            if not self._state.get("managed_session"):
                raise CrawlConfigurationError(
                    "checkpoint's external REL session "
                    f"{state_session} no longer exists"
                )
            self._state["session_id"] = None
            self._save_state()
            _LOGGER.warning(
                "checkpoint session %s no longer exists; creating a replacement",
                state_session,
            )
            return self._create_managed_session(
                previous_session_id=state_session,
                reason="checkpoint session no longer exists",
            )

        _LOGGER.info("checkpoint has no session; creating one")
        return self._create_managed_session()

    def _can_restart_session(self, error: Exception) -> bool:
        if self.requested_session_id is not None or not self._state.get(
            "managed_session"
        ):
            return False
        if isinstance(error, RelTransportError):
            return True
        return isinstance(error, RelRpcError) and error.id in SESSION_RESTART_RPC_IDS

    def _should_restart_after_terminal_failure(
        self, failure_stage: str | None, operation: PageOperation | None
    ) -> bool:
        if (
            self.max_session_restarts == 0
            or self.requested_session_id is not None
            or not self._state.get("managed_session")
        ):
            return False
        if failure_stage == "perform":
            return True
        status = operation.target_http_status if operation is not None else None
        return (
            failure_stage == "http"
            and status is not None
            and (status in {403, 429} or status >= 500)
        )

    def _before_browser_action(self) -> None:
        if self._has_browser_action and self.action_delay > 0:
            _LOGGER.info(
                "pacing browser actions: waiting %.3g seconds",
                self.action_delay,
            )
            time.sleep(self.action_delay)
        self._has_browser_action = True

    def _wait_for_ready(
        self,
        session_id: str,
        selector: str,
        output: Path,
    ) -> PageOperation:
        _LOGGER.info(
            "waiting for selector %r in session %s (timeout %.3g seconds)",
            selector,
            session_id,
            self.timeout,
        )
        self._before_browser_action()
        operation = self.client.perform(
            actions=[{"action": "wait-for", "selector": selector}],
            session_id=session_id,
            output=output,
            timeout=self.timeout,
            wait=self.wait,
        )
        _LOGGER.info("selector %r is ready at %s", selector, operation.url)
        return operation

    @staticmethod
    def _report_skip(message: str) -> None:
        _LOGGER.info("skipping %s", message)

    def _restart_managed_session(self, session_id: str, *, reason: str) -> str:
        if self.requested_session_id is not None or not self._state.get(
            "managed_session"
        ):
            raise CrawlRecoveryError("cannot replace a caller-owned REL session")
        self._state["session_id"] = None
        self._state["pending_session_restart"] = {
            "at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
            "previous_session_id": session_id,
            "reason": reason,
            "delete_error": None,
        }
        self._save_state()
        delete_error: str | None = None
        _LOGGER.info("closing REL session %s: %s", session_id, reason)
        try:
            self.client.delete_session(session_id)
            _LOGGER.info("closed REL session %s", session_id)
        except Exception as error:  # noqa: BLE001 - session cleanup is best effort
            delete_error = f"{type(error).__name__}: {error}"
            _LOGGER.warning(
                "could not close REL session %s: %s",
                session_id,
                delete_error,
            )
            self._state["pending_session_restart"]["delete_error"] = delete_error
            self._save_state()
        return self._create_managed_session(
            previous_session_id=session_id,
            reason=reason,
            delete_error=delete_error,
        )

    def _create_managed_session(
        self,
        *,
        previous_session_id: str | None = None,
        reason: str | None = None,
        delete_error: str | None = None,
    ) -> str:
        pending_restart = self._state.get("pending_session_restart")
        if previous_session_id is None and isinstance(pending_restart, dict):
            pending_previous = pending_restart.get("previous_session_id")
            if isinstance(pending_previous, str):
                previous_session_id = pending_previous
                reason = (
                    pending_restart.get("reason")
                    if isinstance(pending_restart.get("reason"), str)
                    else reason
                )
                delete_error = (
                    pending_restart.get("delete_error")
                    if isinstance(pending_restart.get("delete_error"), str)
                    else delete_error
                )
        _LOGGER.info(
            "creating REL session with profile=%r group=%r",
            self.profile,
            self.group,
        )
        session_id = self.client.create_session(profile=self.profile, group=self.group)
        _LOGGER.info("created REL session %s", session_id)
        self._state["session_id"] = session_id
        self._state["managed_session"] = True
        self._state["pending_session_restart"] = None
        generation = self._state.get("session_generation", 0)
        self._state["session_generation"] = max(generation + 1, 1)
        if previous_session_id is not None:
            self._state["session_restart_count"] += 1
            self._state["last_session_restart"] = {
                "at": datetime.now(UTC).isoformat().replace("+00:00", "Z"),
                "previous_session_id": previous_session_id,
                "session_id": session_id,
                "reason": reason,
                "delete_error": delete_error,
            }
        self._save_state()
        return session_id
