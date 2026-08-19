use serde::Serialize;
use std::env;
use std::fs;
use std::path::Path;

#[derive(Serialize)]
struct Assertion {
    module: usize,
    export: String,
    args: Vec<i64>,
    expected: Vec<i64>,
}

fn extract_modules(src: &str) -> Vec<String> {
    let s = src.as_bytes();
    let mut modules = Vec::new();
    let mut i = 0;
    while i < src.len() {
        if let Some(rel) = src[i..].find("(module") {
            let start = i + rel;
            let mut depth = 0i32;
            let mut j = start;
            while j < s.len() {
                match s[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            modules.push(src[start..j].to_string());
            i = j;
        } else {
            break;
        }
    }
    modules
}

fn parse_i64_consts(s: &str) -> Vec<i64> {
    let mut out = Vec::new();
    let mut rest = s;
    while let Some(p) = rest.find("i64.const") {
        rest = &rest[p + "i64.const".len()..];
        let t = rest.trim_start();
        let mut end = 0;
        let bytes = t.as_bytes();
        if end < bytes.len() && (bytes[end] == b'-' || bytes[end] == b'+') {
            end += 1;
        }
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        if end > 0 {
            if let Ok(v) = t[..end].parse::<i64>() {
                out.push(v);
            }
        }
        rest = &t[end.min(t.len())..];
    }
    out
}

fn parse_assertions(src: &str) -> Vec<Assertion> {
    let mut out = Vec::new();
    let mut module = 0usize;
    let mut seen_module = false;
    let mut pos = 0;
    while pos < src.len() {
        if src[pos..].starts_with("(module") {
            if seen_module {
                module += 1;
            }
            seen_module = true;
            pos += 7;
            continue;
        }
        if src[pos..].starts_with("(assert_return") {
            let start = pos;
            let bytes = src.as_bytes();
            let mut depth = 0i32;
            let mut j = start;
            while j < bytes.len() {
                match bytes[j] {
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            j += 1;
                            break;
                        }
                    }
                    _ => {}
                }
                j += 1;
            }
            let chunk = &src[start..j];
            if let Some(q1) = chunk.find("invoke \"") {
                let rest = &chunk[q1 + 8..];
                if let Some(q2) = rest.find('"') {
                    let export = rest[..q2].to_string();
                    let nums = parse_i64_consts(chunk);
                    if nums.len() >= 2 {
                        let expected = vec![*nums.last().unwrap()];
                        let args = nums[..nums.len() - 1].to_vec();
                        out.push(Assertion {
                            module,
                            export,
                            args,
                            expected,
                        });
                    }
                }
            }
            pos = j;
            continue;
        }
        pos += 1;
    }
    out
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("usage: wastprep <in.wast> <out-dir>");
        std::process::exit(2);
    }
    let src = fs::read_to_string(&args[1]).expect("read wast");
    let out_dir = Path::new(&args[2]);
    fs::create_dir_all(out_dir).unwrap();

    let modules = extract_modules(&src);
    if modules.is_empty() {
        eprintln!("no modules found");
        std::process::exit(1);
    }
    for (i, wat) in modules.iter().enumerate() {
        let wasm = wat::parse_str(wat).unwrap_or_else(|e| {
            eprintln!("wat parse module {i}: {e}");
            std::process::exit(1);
        });
        let path = out_dir.join(format!("mod{i}.wasm"));
        fs::write(&path, wasm).unwrap();
        eprintln!("wrote {} ({} bytes)", path.display(), fs::metadata(&path).unwrap().len());
    }

    let assertions = parse_assertions(&src);
    let json_path = out_dir.join("assertions.json");
    fs::write(&json_path, serde_json::to_string_pretty(&assertions).unwrap()).unwrap();
    eprintln!("wrote {} ({} assertions, {} modules)", json_path.display(), assertions.len(), modules.len());
    if assertions.len() != 60 {
        eprintln!("warning: expected 60 assertions, got {}", assertions.len());
        std::process::exit(1);
    }
}
