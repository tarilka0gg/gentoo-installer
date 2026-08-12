fn main() {
    match installer_core::hardware::Profile::detect() {
        Ok(p) => {
            println!("combo: {}", p.combo());
            println!("candidates:");
            for c in p.candidates() { println!("  {c}"); }
        }
        Err(e) => println!("error: {e}"),
    }
}
