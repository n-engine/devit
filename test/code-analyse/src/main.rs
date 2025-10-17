use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod analyzer;
mod llm;
mod rules;

use analyzer::CodeAnalyzer;
use anyhow::Result;

#[derive(Parser)]
#[command(name = "code-analyzer")]
#[command(about = "Rust implementation of C code analysis tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze C code files against rules
    Analyze {
        /// Path to analyze (file or directory)
        #[arg(short, long)]
        path: PathBuf,
        
        /// Rules file path
        #[arg(short, long, default_value = "rules.txt")]
        rules: PathBuf,
        
        /// Output format (text, json)
        #[arg(short, long, default_value = "text")]
        format: String,
        
        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },
    
    /// Make LLM request (wrapper for various providers)
    Request {
        /// Prompt text
        prompt: String,
        
        /// Input file to include
        #[arg(short, long)]
        file: Option<PathBuf>,
        
        /// Model to use
        #[arg(short, long, default_value = "llama3")]
        model: String,
        
        /// Output raw response
        #[arg(short, long)]
        raw: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    
    match cli.command {
        Commands::Analyze { path, rules, format, verbose } => {
            let analyzer = CodeAnalyzer::new(rules)?;
            let results = analyzer.analyze_path(&path, verbose)?;
            
            match format.as_str() {
                "json" => println!("{}", serde_json::to_string_pretty(&results)?),
                _ => analyzer.print_results(&results),
            }
        }
        
        Commands::Request { prompt, file, model, raw } => {
            let llm_client = llm::LlmClient::new(&model);
            let response = llm_client.request(&prompt, file.as_deref()).await?;
            
            if raw {
                print!("{}", response.raw);
            } else {
                println!("{}", response.content);
            }
        }
    }
    
    Ok(())
}