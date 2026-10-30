//! A stand-in for `metallic-player.exe` that tests and manual checks start instead of the real
//! Player. It writes each argument it received on its own line to `<its own path>.args.txt`,
//! so a check can see exactly what the Launcher passed.

use std::io::Write;

fn main() {
    let Ok(exe) = std::env::current_exe() else {
        std::process::exit(2);
    };
    let mut out = exe.into_os_string();
    out.push(".args.txt");
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(out)
    else {
        std::process::exit(3);
    };
    for arg in std::env::args_os().skip(1) {
        let _ = writeln!(file, "{}", arg.to_string_lossy());
    }
}
