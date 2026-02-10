use std::env;
use std::ffi::OsString;
use std::process::Command;
use std::os::unix::process::CommandExt;

fn main() {
    // Determine which component to exec (env COMPONENT or default)
    let component = env::var_os("COMPONENT").unwrap_or_else(|| OsString::from("hpfeeds-server"));

    // Convert to string
    let comp_str = component.to_string_lossy().into_owned();

    // Resolve binary path - try /usr/local/bin/<component> first, then rely on PATH
    let bin_path = format!("/usr/local/bin/{}", comp_str);

    // Collect args passed to entrypoint (after the binary name)
    let args: Vec<OsString> = env::args_os().skip(1).collect();

    // Try executing the resolved path; fallback to just component name.
    let err = Command::new(&bin_path).args(&args).exec();

    // If exec failed, try fallback to component in PATH
    let err2 = Command::new(&comp_str).args(&args).exec();

    // If we reach here both exec calls failed (they call _exit on success), so print errors and exit
    eprintln!("Failed to exec {}: {:?}", bin_path, err);
    eprintln!("Failed to exec {}: {:?}", comp_str, err2);
    std::process::exit(1);
}
