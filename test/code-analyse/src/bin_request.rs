use clap::Parser;
use code_analyzer::llm::LlmClient;
use std::io::{self, Read};
use std::path::PathBuf;
use anyhow::Result;

#[derive(Parser)]
#[command(name = "request")]
#[command(about = "LLM request wrapper - equivalent to bash request script")]
struct Args {
    /// Prompt text (if not provided, reads from stdin)
    prompt: Option<String>,
    
    /// Input file to include in prompt
    #[arg(short, long)]
    file: Option<PathBuf>,
    
    /// Model to use
    #[arg(short, long, default_value = "llama3")]
    model: String,
    
    /// Output raw JSON response
    #[arg(short, long)]
    raw: bool,
    
    /// Verbose output
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Get prompt from args or stdin
    let prompt = match args.prompt {
        Some(p) => p,
        None => {
            if args.verbose {
                eprintln!("Reading prompt from stdin...");
            }
            let mut stdin = io::stdin();
            let mut buffer = String::new();
            stdin.read_to_string(&mut buffer)?;
            buffer.trim().to_string()
        }
    };
    
    if prompt.is_empty() {
        eprintln!("Error: No prompt provided");
        std::process::exit(1);
    }
    
    if args.verbose {
        eprintln!("Model: {}", args.model);
        eprintln!("Prompt length: {} chars", prompt.len());
        if let Some(ref file) = args.file {
            eprintln!("Including file: {}", file.display());
        }
    }
    
    let client = LlmClient::new(&args.model);
    let response = client.request(&prompt, args.file.as_deref()).await?;
    
    if args.raw {
        print!("{}", response.raw);
    } else {
        println!("{}", response.content);
        
        if args.verbose {
            if let Some(tokens) = response.tokens_used {
                eprintln!("\nTokens used: {}", tokens);
            }
        }
    }
    
    Ok(())
}