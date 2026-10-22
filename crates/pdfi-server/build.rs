// Re-embed the web UI whenever its build output changes.
fn main() {
    println!("cargo:rerun-if-changed=../../apps/web/dist");
}
