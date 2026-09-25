use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tracing::{debug, error};

#[derive(Debug, Serialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessageResponse,
}

#[derive(Debug, Deserialize)]
struct ChatMessageResponse {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Clone)]
pub struct FreeTokenClient {
    http: Client,
    base_url: String,
    default_model: String,
    max_tokens: u32,
    temperature: f32,
    api_key: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelListResponse {
    data: Option<Vec<ModelItem>>,
}

#[derive(Debug, Deserialize)]
struct ModelItem {
    id: String,
}

impl FreeTokenClient {
    pub fn new(base_url: String, default_model: String, max_tokens: u32, temperature: f32, api_key: Option<String>) -> Self {
        let http = Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .unwrap_or_default();

        Self {
            http,
            base_url,
            default_model,
            max_tokens,
            temperature,
            api_key,
        }
    }

    fn apply_auth(&self, mut req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            if !key.trim().is_empty() {
                req = req.header("Authorization", format!("Bearer {}", key.trim()));
            }
        }
        req
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    pub fn default_model(&self) -> &str {
        &self.default_model
    }

    /// Voert een prompt uit via de OpenAI-compatibele FreeToken API
    pub async fn ask(&self, user: &str, prompt: &str, custom_model: Option<&str>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let model = custom_model.unwrap_or(&self.default_model);

        let payload = ChatCompletionRequest {
            model: model.to_string(),
            messages: vec![
                ChatMessage {
                    role: "system".into(),
                    content: "Je bent een behulpzame, nuchtere en gevatte AI-assistent in een hybride IRC/Discord chatkanaal. "
                        .to_string()
                        + "Antwoord beknopt, to-the-point en zonder overbodige omhaal.",
                },
                ChatMessage {
                    role: "user".into(),
                    content: format!("{}: {}", user, prompt),
                },
            ],
            max_tokens: self.max_tokens,
            temperature: self.temperature,
        };

        debug!("Verzenden prompt naar FreeToken ({}) voor gebruiker {}", url, user);

        let req = self.http.post(&url).json(&payload);
        let resp = self.apply_auth(req).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_body = resp.text().await.unwrap_or_default();
            error!("FreeToken API error ({}): {}", status, err_body);
            return Err(format!("FreeToken fout {}: {}", status, err_body).into());
        }

        let body: ChatCompletionResponse = resp.json().await?;

        if let Some(choice) = body.choices.into_iter().next() {
            Ok(choice.message.content.trim().to_string())
        } else {
            Ok("Geen reactie ontvangen van het AI model.".to_string())
        }
    }

    /// Snelle test om te controleren of de FreeToken service bereikbaar is
    pub async fn ping(&self) -> bool {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let req = self.http.get(&url).timeout(Duration::from_secs(3));
        match self.apply_auth(req).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        }
    }

    /// Haalt de lijst van beschikbare modellen op bij de lokale server via GET /v1/models
    pub async fn list_models(&self) -> Result<Vec<String>, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/models", self.base_url.trim_end_matches('/'));
        let req = self.http.get(&url).timeout(Duration::from_secs(5));
        let resp = self.apply_auth(req).send().await?;

        if !resp.status().is_success() {
            return Err(format!("Kon modellen niet ophalen (status {})", resp.status()).into());
        }

        let body: ModelListResponse = resp.json().await?;
        let models = body
            .data
            .unwrap_or_default()
            .into_iter()
            .map(|m| m.id)
            .collect();

        Ok(models)
    }

    /// Genereert een beknopte 1-regel omschrijving van een afbeelding via een Vision model
    pub async fn describe_image(&self, image_url: &str, custom_model: Option<&str>) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/chat/completions", self.base_url.trim_end_matches('/'));
        let model = custom_model.unwrap_or(&self.default_model);

        // OpenAI vision payload
        let payload = serde_json::json!({
            "model": model,
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Beschrijf deze afbeelding in het Nederlands in exact 1 korte, feitelijke zin (maximaal 120 tekens) voor weergave op IRC chat:"
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": image_url
                            }
                        }
                    ]
                }
            ],
            "max_tokens": 80,
            "temperature": 0.2
        });

        let req = self.http.post(&url).json(&payload).timeout(Duration::from_secs(15));
        let resp = self.apply_auth(req).send().await?;

        if !resp.status().is_success() {
            return Err(format!("Vision model fout (status {})", resp.status()).into());
        }

        let body: ChatCompletionResponse = resp.json().await?;
        if let Some(choice) = body.choices.into_iter().next() {
            let desc = choice.message.content.trim().trim_matches('"').to_string();
            Ok(desc)
        } else {
            Err("Geen beschrijving ontvangen".into())
        }
    }
}
