fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(funget_cli::run(std::env::args_os()) as u8)
}
