use modo::Result;

fn main() -> Result<()> {
    let args = wild::args_os();
    modo::run(args)
}
