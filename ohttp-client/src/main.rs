use bhttp::{Message, Mode};
use std::fs::File;
use std::io;
use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;
use structopt::StructOpt;

type Res<T> = Result<T, Box<dyn std::error::Error>>;

#[derive(Debug)]
struct HexArg(Vec<u8>);
impl FromStr for HexArg {
    type Err = hex::FromHexError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        hex::decode(s).map(HexArg)
    }
}
impl Deref for HexArg {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[derive(Debug, StructOpt)]
#[structopt(name = "ohttp-client", about = "Make an oblivious HTTP request.")]
struct Args {
    /// The URL of an oblivious proxy resource.
    /// If you use an oblivious request resource, this also works, though
    /// you don't get any of the privacy guarantees.
    url: String,
    /// A hexadecimal version of the key configuration for the target URL.
    config: HexArg,

    /// Where to read request content.
    /// If you omit this, input is read from `stdin`.
    #[structopt(long, short = "i")]
    input: Option<PathBuf>,

    /// Where to write response content.
    /// If you omit this, output is written to `stdout`.
    #[structopt(long, short = "o")]
    output: Option<PathBuf>,

    /// Read and write as binary HTTP messages instead of text.
    #[structopt(long, short = "b")]
    binary: bool,

    /// When creating message/bhttp, use the indefinite-length form.
    #[structopt(long, short = "n")]
    indefinite: bool,

    /// Enable override for the trust store.
    #[structopt(long)]
    trust: Option<PathBuf>,
}

impl Args {
    fn mode(&self) -> Mode {
        if self.indefinite {
            Mode::IndefiniteLength
        } else {
            Mode::KnownLength
        }
    }
}

#[tokio::main]
async fn main() -> Res<()> {
    let args = Args::from_args();
    ::ohttp::init();

    let mut input: Box<dyn io::BufRead> = if let Some(infile) = &args.input {
        Box::new(io::BufReader::new(File::open(infile)?))
    } else {
        Box::new(io::BufReader::new(std::io::stdin()))
    };
    let request = if args.binary {
        Message::read_bhttp(&mut input)?
    } else {
        Message::read_http(&mut input)?
    };

    let mut request_buf = Vec::new();
    request.write_bhttp(Mode::KnownLength, &mut request_buf)?;
    let ohttp_req = ohttp::ClientRequest::new(&args.config)?;
    let (enc_request, ohttp_res) = ohttp_req.encapsulate(&request_buf)?;
    println!("Request: {}", hex::encode(&enc_request));

    let mut tls_config = rustls::ClientConfig::new();
    if let Some(pem) = &args.trust {
        tls_config
            .root_store
            .add_pem_file(&mut io::BufReader::new(File::open(pem)?))
            .expect("Trust store read failed");
    }
    let client = reqwest::ClientBuilder::new()
        .use_preconfigured_tls(tls_config)
        .build()?;
    let enc_response = client
        .post(&args.url)
        .header("content-type", "message/ohttp-req")
        .body(enc_request)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    println!("Response: {}", hex::encode(&enc_response));
    let response_buf = ohttp_res.decapsulate(&enc_response)?;
    let response = Message::read_bhttp(&mut std::io::BufReader::new(&response_buf[..]))?;

    let mut output: Box<dyn io::Write> = if let Some(outfile) = &args.output {
        Box::new(File::open(outfile)?)
    } else {
        Box::new(std::io::stdout())
    };
    if args.binary {
        response.write_bhttp(args.mode(), &mut output)?;
    } else {
        response.write_http(&mut output)?;
    }
    Ok(())
}
