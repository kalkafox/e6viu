use std::{
    env,
    fs::File,
    io::{stdout, Read, Write},
    path::Path,
    sync::Arc,
    time::Duration,
};

use base64::Engine;
use clap::Parser;
use clap_derive::Parser;
use crossterm::{
    cursor::{self, MoveToColumn},
    event::{self, Event, KeyCode, KeyEvent},
    terminal::{disable_raw_mode, enable_raw_mode},
    ExecutableCommand,
};
use dotenv::dotenv;
use e6viu::{E621Posts, Post, Spinners};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use rand::seq::IteratorRandom;
use reqwest::header::{HeaderValue, AUTHORIZATION, USER_AGENT};
use tokio::task::JoinHandle;
use viuer::{get_kitty_support, KittySupport};

/// e6viu - Display random e6 images on the terminal
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    /// Tags to use (split with commas, e.g fox,canine,score:>50)
    #[arg(short, long, default_value = "fox")]
    tags: String,

    /// Paws only.
    #[arg(short, long)]
    paws: bool,

    /// Whether to include cubs. (Defaults to false)
    #[arg(short, long)]
    cubs: bool,
}

const E6_URL: &str = "https://e621.net/posts.json?limit=1";

struct Spinner {
    spin_task: Option<JoinHandle<()>>,
    spinners: Spinners,
}

impl Spinner {
    async fn new() -> Result<Spinner, Box<dyn std::error::Error>> {
        Ok(Spinner {
            spin_task: None,
            spinners: get_spinners().await?,
        })
    }

    fn spin(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let mut rng = rand::thread_rng();

        let spinner = self
            .spinners
            .iter()
            .choose(&mut rng)
            .ok_or("No spinner found, somehow")?;

        let frames = spinner.1.frames.to_owned();

        let interval = spinner.1.interval.try_into().unwrap();

        stdout().execute(cursor::Hide)?;

        self.spin_task = Some(tokio::spawn(async move {
            loop {
                for i in frames.iter() {
                    print!("{}", i);
                    stdout().execute(MoveToColumn(0)).unwrap();
                    tokio::time::sleep(Duration::from_millis(interval)).await;
                }
            }
        }));

        Ok(())
    }

    fn stop(&self) {
        if self.spin_task.is_none() {
            return;
        }

        self.spin_task.as_ref().unwrap().abort();
        stdout().execute(cursor::Show).unwrap();
    }
}

struct E621;

impl E621 {
    async fn new(tags: Arc<Vec<String>>) -> Result<Vec<Post>, Box<dyn std::error::Error>> {
        let tags = tags.join("+");

        let client = reqwest::Client::new();

        let mut headers = reqwest::header::HeaderMap::new();

        headers.append(
            USER_AGENT,
            HeaderValue::from_static("e6viu / made by Kalka"),
        );

        let token = env::var("E621_TOKEN");

        match token {
            Ok(token) => {
                let encoding =
                    base64::engine::general_purpose::STANDARD.encode(format!("kalka:{}", token));

                let t = format!("Basic {}", encoding);
                let header_value = HeaderValue::from_str(&t).expect("Invalid header value");

                headers.append(AUTHORIZATION, header_value);
            }
            Err(why) => {
                println!("Warning: Missing e6 token, create an .env file with the environment variable E621_TOKEN for api key support - {why:?}")
            }
        }

        let res = client
            .get(format!(
                "{}&tags=order:random+-female+-intersex+{}",
                E6_URL, tags
            ))
            .headers(headers)
            .send()
            .await?;

        let data = res.json::<E621Posts>().await?;

        if data.posts.is_empty() {
            return Err("Posts is empty".into());
        }

        Ok(data.posts)
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenv().ok();

    let args = Args::parse();

    println!("{}", args.tags);

    if get_kitty_support() == KittySupport::None {
        println!("Results are best viewed using the kitty terminal.");
    }

    let mut tags = "score:>100,fox";

    if args.paws {
        tags = "score:>100,paws,pawpads";
    }

    let mut tags = tags
        .split(",")
        .map(|s| s.to_owned())
        .collect::<Vec<String>>();

    if !args.cubs {
        tags.push("-cub".to_owned());
    }

    let mut downloaded = 0;

    let tags = Arc::new(tags);

    loop {
        let mut spinner = Spinner::new().await?;

        spinner.spin()?;

        let posts = E621::new(tags.clone()).await?;

        spinner.stop();

        let post = posts.first().ok_or("No posts found, somehow")?;

        match post.file.ext {
            e6viu::Ext::Webm => continue,
            e6viu::Ext::Swf => continue,
            _ => {}
        }

        let total_size = post.file.size;

        let pb = ProgressBar::new(total_size.try_into()?);

        pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{bar:.cyan/blue}] {bytes}/{total_bytes} | {bytes_per_sec}")?
            .progress_chars("#>-"));

        let conf = viuer::Config {
            absolute_offset: false,
            ..Default::default()
        };

        let url = post.file.url.to_owned();

        let res = reqwest::get(url).await?;

        let mut data = res.bytes_stream();

        let filename = "/tmp/e6-file";

        let mut file = File::create(filename)?;

        while let Some(chunk) = data.next().await {
            let data = chunk?;

            let chunk_size: u64 = data.len().try_into()?;

            let new = std::cmp::min(downloaded + chunk_size, total_size.try_into()?);

            downloaded = new;

            pb.set_position(new);

            file.write_all(&data)?;
        }

        downloaded = 0;

        //spinner.stop();

        viuer::print_from_file(filename, &conf)?;

        std::fs::remove_file("/tmp/e6-file")?;

        println!("{}", post.file.url);
        println!("https://e621.net/posts/{}", post.id);

        println!("Press N for a new image, or Q to quit");

        enable_raw_mode()?;

        loop {
            if let Event::Key(KeyEvent { code, .. }) = event::read()? {
                match code {
                    KeyCode::Char(c) => {
                        stdout().execute(MoveToColumn(0))?;

                        if c == 'n' {
                            disable_raw_mode()?;
                            break;
                        }

                        if c == 'q' {
                            disable_raw_mode()?;
                            std::process::exit(0);
                        }
                    }
                    KeyCode::Esc => {
                        println!("Escape pressed, exiting.");
                        stdout().execute(MoveToColumn(0))?;
                        disable_raw_mode()?;
                        std::process::exit(0);
                    }
                    _ => {}
                }
            }
        }
    }
}

async fn get_spinners() -> Result<Spinners, Box<dyn std::error::Error>> {
    let spinners_filename = "/tmp/spinners.json";

    if !Path::new(spinners_filename).exists() {
        let res = reqwest::get(
            "https://raw.githubusercontent.com/sindresorhus/cli-spinners/main/spinners.json",
        )
        .await?;

        let mut spinners_file = File::create(spinners_filename)?;

        let mut spinners_file_stream = res.bytes_stream();

        while let Some(chunk) = spinners_file_stream.next().await {
            spinners_file.write_all(&chunk?)?;
        }
    }

    let mut spinners_file = File::open(spinners_filename)?;

    let mut spinners_raw = String::new();

    spinners_file.read_to_string(&mut spinners_raw)?;

    let spinners = serde_json::from_str::<Spinners>(&spinners_raw)?;

    Ok(spinners)
}
