fn main() -> std::process::ExitCode {
    std::process::ExitCode::from(funget::run(std::env::args_os()) as u8)
}
