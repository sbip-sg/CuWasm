use std::env;
use std::fs;
use std::process;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wat2wasm <input.wat> <output.wasm>");
        process::exit(2);
    }
    let wat = fs::read_to_string(&args[1]).unwrap_or_else(|e| {
        eprintln!("read {}: {e}", args[1]);
        process::exit(1);
    });
    let wasm = wat::parse_str(&wat).unwrap_or_else(|e| {
        eprintln!("parse wat: {e}");
        process::exit(1);
    });
    fs::write(&args[2], wasm).unwrap_or_else(|e| {
        eprintln!("write {}: {e}", args[2]);
        process::exit(1);
    });
}
