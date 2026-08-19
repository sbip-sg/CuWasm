use std::env;
use wasmi::{Config, Engine, Linker, Module, Store};

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: cuwasm-oracle <module.wasm> <export> [i64 args...]");
        std::process::exit(2);
    }
    let wasm = std::fs::read(&args[1]).unwrap_or_else(|e| {
        eprintln!("read: {e}");
        std::process::exit(1);
    });
    let export = &args[2];
    let mut invoke_args: Vec<i64> = Vec::new();
    for a in &args[3..] {
        invoke_args.push(a.parse::<i64>().unwrap_or_else(|_| {
            eprintln!("bad arg {a}");
            std::process::exit(2);
        }));
    }
    if invoke_args.len() != 1 {
        eprintln!("stage-1 oracle expects a single i64 argument");
        std::process::exit(2);
    }
    let n = invoke_args[0];

    let mut config = Config::default();
    config.wasm_tail_call(true);
    let engine = Engine::new(&config);
    let module = match Module::new(&engine, &wasm) {
        Ok(m) => m,
        Err(e) => {
            println!(
                "{{\"status\": \"unsupported_op\", \"results\": [], \"error\": {:?}}}",
                e.to_string()
            );
            std::process::exit(1);
        }
    };
    let mut store = Store::new(&engine, ());
    let linker = Linker::new(&engine);
    let instance = match linker.instantiate(&mut store, &module) {
        Ok(i) => match i.start(&mut store) {
            Ok(s) => s,
            Err(e) => {
                println!(
                    "{{\"status\": \"trap_unreachable\", \"results\": [], \"error\": {:?}}}",
                    e.to_string()
                );
                std::process::exit(1);
            }
        },
        Err(e) => {
                println!(
                    "{{\"status\": \"trap_unreachable\", \"results\": [], \"error\": {:?}}}",
                    e.to_string()
                );
            std::process::exit(1);
        }
    };

    let func = match instance.get_typed_func::<i64, i64>(&store, export) {
        Ok(f) => f,
        Err(e) => {
            println!(
                "{{\"status\": \"unsupported_op\", \"results\": [], \"error\": {:?}}}",
                e.to_string()
            );
            std::process::exit(1);
        }
    };

    match func.call(&mut store, n) {
        Ok(v) => {
            println!("{{\"status\": \"ok\", \"results\": [{v}]}}");
        }
        Err(e) => {
            println!(
                "{{\"status\": \"trap_unreachable\", \"results\": [], \"error\": {:?}}}",
                e.to_string()
            );
            std::process::exit(1);
        }
    }
}
