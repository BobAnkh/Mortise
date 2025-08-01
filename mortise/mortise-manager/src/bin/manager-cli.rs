use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use futures::{SinkExt, StreamExt};
use mortise_common::{read_be_u32, ManagerOperation, Operation};
use tokio::net::{
    unix::{ReadHalf, WriteHalf},
    UnixStream,
};
use tokio_util::codec::{FramedRead, FramedWrite, LengthDelimitedCodec};

#[derive(Parser, Debug)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    #[arg(skip = false)]
    interactive_mode: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Load bpf struct_ops into kernel
    Load(LoadArgs),
    /// Unload bpf struct_ops from kernel
    Unload(UnloadArgs),
    /// Insert bpf struct_ops into kernel
    Insert(InsertArgs),
    /// Send ping-pong to manager
    Ping,
    /// Enter interactive mode
    Interactive,
    /// Exit interactive mode
    Quit,
}

#[derive(Args, Debug)]
struct LoadArgs {
    path: String,
}

#[derive(Args, Debug)]
struct UnloadArgs {
    obj_id: u32,
}

#[derive(Args, Debug)]
struct InsertArgs {
    obj_id: u32,
    path: String,
}

async fn handle_command(
    mut cli: Cli,
    writer: &mut FramedWrite<WriteHalf<'_>, LengthDelimitedCodec>,
    reader: &mut FramedRead<ReadHalf<'_>, LengthDelimitedCodec>,
) -> Result<u32> {
    match cli.command {
        Commands::Load(args) => {
            let path = std::path::Path::new(&args.path);
            let path = path.canonicalize()?;
            let path = path.display().to_string();
            let req: Operation = ManagerOperation::Load { path, option: None }.into();
            let req_bytes = serde_json::to_vec(&req).map(Into::into)?;
            writer.send(req_bytes).await?;
            let resp_bytes = reader.next().await.unwrap()?;
            let resp: std::result::Result<Vec<u8>, String> =
                serde_json::from_slice(resp_bytes.as_ref())?;
            match resp {
                Ok(r) => {
                    let mut b = r.as_ref();
                    let obj_id = read_be_u32(&mut b);
                    println!("Loaded with obj_id {obj_id}")
                }
                Err(e) => println!("Failed to load: {e}"),
            }
        }
        Commands::Unload(args) => {
            let req: Operation = ManagerOperation::Unload {
                obj_id: args.obj_id,
            }
            .into();
            let req_bytes = serde_json::to_vec(&req).map(Into::into)?;
            writer.send(req_bytes).await?;
            let resp_bytes = reader.next().await.unwrap()?;
            let resp: std::result::Result<Vec<u8>, String> =
                serde_json::from_slice(resp_bytes.as_ref())?;
            match resp {
                Ok(_) => println!("Unloaded object with id {}", args.obj_id),
                Err(e) => println!("Failed to unload: {e}"),
            }
        }
        Commands::Insert(args) => {
            let path = std::path::Path::new(&args.path);
            let path = path.canonicalize()?;
            let path = path.display().to_string();
            let req: Operation = ManagerOperation::Insert {
                obj_id: args.obj_id,
                path,
                option: None,
            }
            .into();
            let req_bytes = serde_json::to_vec(&req).map(Into::into)?;
            writer.send(req_bytes).await?;
            let resp_bytes = reader.next().await.unwrap()?;
            let resp: std::result::Result<Vec<u8>, String> =
                serde_json::from_slice(resp_bytes.as_ref())?;
            match resp {
                Ok(_) => println!("Inserted object with id {}", args.obj_id),
                Err(e) => println!("Failed to insert: {e}"),
            }
        }
        Commands::Ping => {
            println!("Ping");
            let req: Operation = ManagerOperation::PingPong.into();
            let req_bytes = serde_json::to_vec(&req).map(Into::into)?;
            writer.send(req_bytes).await?;
            let resp_bytes = reader.next().await.unwrap()?;
            let resp: std::result::Result<Vec<u8>, String> =
                serde_json::from_slice(resp_bytes.as_ref())?;
            let resp = match resp {
                Ok(_) => "Pong".to_string(),
                Err(e) => e,
            };
            println!("{resp}");
        }
        Commands::Quit => {
            if cli.interactive_mode {
                cli.interactive_mode = false;
                return Ok(1);
            }
        }
        _ => {}
    }
    Ok(0)
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut stream = UnixStream::connect("/tmp/mortise.sock").await?;
    let (rh, wh) = stream.split();
    let mut reader = LengthDelimitedCodec::builder()
        .length_field_offset(0) // default value
        .length_field_type::<u32>()
        .length_adjustment(0) // default value
        .new_read(rh);
    let mut writer = LengthDelimitedCodec::builder()
        .length_field_type::<u32>()
        .new_write(wh);
    if let Commands::Interactive = cli.command {
        let mut rl = clap_repl::ClapEditor::<Cli>::new();
        loop {
            let Some(mut cmd) = rl.read_command() else {
                continue;
            };
            cmd.interactive_mode = true;
            let res = handle_command(cmd, &mut writer, &mut reader).await?;
            if res != 0 {
                break;
            }
        }
    } else {
        handle_command(cli, &mut writer, &mut reader).await?;
    }
    Ok(())
}
