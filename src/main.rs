//! Binary entrypoint for the Lithograph CLI.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    lithograph::run()
}
