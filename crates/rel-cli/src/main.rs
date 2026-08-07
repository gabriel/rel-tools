use std::ffi::OsString;

fn main() {
    std::process::exit(rel_cli::main_exit_code(
        std::env::args_os().skip(1).collect::<Vec<OsString>>(),
    ));
}
