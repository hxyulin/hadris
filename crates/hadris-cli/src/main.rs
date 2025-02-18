use clap::Parser;

#[derive(Debug, Parser)]
struct Arguments {
    #[clap(value_parser)]
    input: clio::Input,
    #[clap(subcommand)]
    subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    Read,
    Write,
}

fn main() {
    use hadris::FileSystem;
    let args = Arguments::parse();
    let mut bytes = Vec::with_capacity(512 * 100000);
    bytes.resize(512 * 100000, 0);
    let mut fs = FileSystem::with_bytes(hadris::FileSystemType::Fat, &mut bytes);
    let file = fs.open_file("test.txt", hadris::OpenMode::Write).unwrap();
    file.write(&mut fs, &[b'A'; 256]).unwrap();

    // Write the bytes
    drop(file);
    drop(fs);
    std::fs::write("target/test.img", &bytes).unwrap();
}
