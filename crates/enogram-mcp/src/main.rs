use enogram_mcp::{serve, Config, Server};
use std::io;
use std::process::ExitCode;

fn main() -> ExitCode {
    let server = Config::from_args(std::env::args_os().skip(1)).and_then(Server::new);
    let mut server = match server {
        Ok(server) => server,
        Err(error) => {
            eprintln!("enogram-mcp: {error}");
            eprintln!("Usage: enogram-mcp --repo <absolute checkout root> [--repo <root> ...] [--allow-write]");
            return ExitCode::from(2);
        }
    };
    match serve(&mut server, io::stdin().lock(), io::stdout().lock()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => {
            // Transport errors may occur after a write; never repeat that write.
            eprintln!("enogram-mcp: stdio transport closed or failed; inspect repository state before further writes.");
            ExitCode::from(1)
        }
    }
}
