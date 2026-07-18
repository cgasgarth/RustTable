fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.as_slice() == [String::from("--version")] {
        println!("RustTable {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if !arguments.is_empty() {
        eprintln!("unsupported arguments: {}", arguments.join(" "));
        std::process::exit(2);
    }
    if let Err(error) = rusttable_app::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
