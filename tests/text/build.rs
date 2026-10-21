use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=generate_fonts.py");
    println!("cargo:rerun-if-changed=generate_named_instances.py");
    println!("cargo:rerun-if-changed=requirements.txt");
    println!("cargo:rerun-if-env-changed=PYTHON");
    let directory = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo OUT_DIR")).join("fonts");
    let python = env::var_os("PYTHON").unwrap_or_else(|| "python3".into());
    for generator in ["generate_fonts.py", "generate_named_instances.py"] {
        let status = Command::new(&python)
            .arg(generator)
            .arg(&directory)
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .status()
            .expect("run Python font generator; install Python 3 and tests/text/requirements.txt");
        assert!(
            status.success(),
            "{generator} failed; install tests/text/requirements.txt with the selected Python"
        );
    }
}
