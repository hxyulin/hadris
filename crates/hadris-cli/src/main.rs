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
    let mut bytes = Vec::with_capacity(512 * 1024);
    bytes.resize(512 * 1024, 0);
    let mut fs = hadris::fat::FileSystem::new_f32(hadris::fat::structures::Fat32Ops::recommended_config_for(1024), &mut bytes);
    let data = b"Hello, world!";
    fs.create_file("test.txt", data);
    let file = fs.open("test.txt", hadris::OpenMode::Read).unwrap();
    let mut buf = [0u8; 512];
    file.read(&fs, &mut buf).unwrap();
    println!("{:?}", file);
    println!("fs: {:?}", fs);
    println!("buf: {:?}", buf)
}
