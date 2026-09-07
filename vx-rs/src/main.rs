mod detect;
mod util;
mod vx;
mod vxr;

use std::env;
use std::path::Path;

fn main() {
    let argv: Vec<String> = env::args().collect();
    let bin = argv
        .first()
        .map(|a| {
            Path::new(a)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("vx")
                .to_string()
        })
        .unwrap_or_else(|| "vx".to_string());

    // Multicall: `vxr ...` ejecuta, cualquier otro nombre gestiona.
    let code = if bin == "vxr" {
        vxr::main(&argv[1..])
    } else {
        vx::main(&argv[1..])
    };
    std::process::exit(code);
}
