use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Serialize, Deserialize)]
pub struct LlmResponse {
    pub content: String,
    pub raw: String,
    pub model: String,
    pub tokens_used: Option<u32>,
}

pub struct LlmClient {
    model: String,
    base_url: String,
}

impl LlmClient {
    pub fn new(model: &str) -> Self {
        let base_url = std::env::var("OLLAMA_HOST")
            .unwrap_or_else(|_| "http://localhost:11434".to_string());
        
        Self {
            model: model.to_string(),
            base_url,
        }
    }
    
    pub async fn request(&self, prompt: &str, file_path: Option<&Path>) -> Result<LlmResponse> {
        let mut full_prompt = prompt.to_string();
        
        // Append file content if provided
        if let Some(path) = file_path {
            let file_content = fs::read_to_string(path)
                .context(format!("Failed to read file: {}", path.display()))?;
            
            full_prompt.push_str(&format!(
                "\n\nFile content from '{}':\n```\n{}\n```",
                path.display(),
                file_content
            ));
        }
        
        // In a real implementation, this would call Ollama API
        // For now, we'll return a mock response
        self.mock_request(&full_prompt).await
    }
    
    async fn mock_request(&self, prompt: &str) -> Result<LlmResponse> {
        // Mock implementation - in real code this would be HTTP request to Ollama
        let response_content = if prompt.contains("analyze") {
            format!("Mock analysis response for model '{}':\n\nThe code appears to follow standard C conventions. Here are some observations:\n- Function definitions are properly formatted\n- No obvious security vulnerabilities detected\n- Consider adding more comments for complex logic\n\nPrompt was: {}", self.model, prompt.chars().take(100).collect::<String>())
        } else {
            format!("Mock response from model '{}' for prompt: {}", 
                   self.model, 
                   prompt.chars().take(50).collect::<String>())
        };
        
        Ok(LlmResponse {
            content: response_content.clone(),
            raw: format!("{{\"response\": \"{}\"}}", response_content.replace('"', "\\\"")),
            model: self.model.clone(),
            tokens_used: Some(prompt.len() as u32 / 4), // rough estimate
        })
    }
    
    #[allow(dead_code)]
    async fn real_ollama_request(&self, prompt: &str) -> Result<LlmResponse> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/generate", self.base_url);
        
        let request_body = serde_json::json!({
            "model": self.model,
            "prompt": prompt,
            "stream": false
        });
        
        let response = client
            .post(&url)
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to Ollama")?;
        
        let response_text = response
            .text()
            .await
            .context("Failed to read response from Ollama")?;
        
        // Parse Ollama response
        let parsed: serde_json::Value = serde_json::from_str(&response_text)
            .context("Failed to parse Ollama response")?;
        
        let content = parsed["response"]
            .as_str()
            .unwrap_or("No response content")
            .to_string();
        
        Ok(LlmResponse {
            content,
            raw: response_text,
            model: self.model.clone(),
            tokens_used: None, // Ollama doesn't provide token count in this format
        })
    }
}