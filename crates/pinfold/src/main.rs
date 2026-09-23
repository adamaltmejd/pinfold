use pinfold::init;

fn main() {
    // The one binary doubles as PID 1 in a box on Linux.
    if std::env::args().nth(1).as_deref() == Some("init") {
        init::run();
    }
    println!("pinfold {}", env!("CARGO_PKG_VERSION"));
}
