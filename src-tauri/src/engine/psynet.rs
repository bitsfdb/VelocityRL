use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose};

type HmacSha256 = Hmac<Sha256>;

const REQUEST_KEY: &str = "c338bd36fb8c42b1a431d30add939fc7";
const PSYNET_RPC_URL: &str = "https://api.rlpp.psynet.gg/rpc/";

pub fn get_psysig(body: &str) -> String {
    let key = hex::decode(REQUEST_KEY).unwrap();
    let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC can take key of any size");
    mac.update(format!("-{}", body).as_bytes());
    let result = mac.finalize();
    general_purpose::STANDARD.encode(result.into_bytes())
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PsynetResponse<T> {
    #[serde(rename = "Result")]
    pub result: T,
}

pub async fn call_rpc<T: serde::de::DeserializeOwned>(
    service: &str,
    body: &serde_json::Value,
    psy_token: Option<&str>,
    session_id: Option<&str>,
) -> Result<T, String> {
    let client = reqwest::Client::new();
    let json_body = serde_json::to_string(body).map_err(|e| e.to_string())?;
    let sig = get_psysig(&json_body);

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("PsyService", service.parse().unwrap());
    headers.insert("PsyEnvironment", "Prod".parse().unwrap());
    headers.insert("User-Agent", "RL Win/250811.43331.492665 gzip".parse().unwrap());
    headers.insert("Content-Type", "application/json".parse().unwrap());
    headers.insert("PsySig", sig.parse().unwrap());

    if let Some(token) = psy_token {
        headers.insert("PsyToken", token.parse().unwrap());
    }
    if let Some(id) = session_id {
        headers.insert("PsySessionID", id.parse().unwrap());
    }

    let resp = client.post(PSYNET_RPC_URL)
        .headers(headers)
        .body(json_body)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if resp.status() != 200 {
        return Err(format!("RPC failed: {} - {}", resp.status(), resp.text().await.unwrap_or_default()));
    }

    let data: PsynetResponse<T> = resp.json().await.map_err(|e| e.to_string())?;
    Ok(data.result)
}

pub async fn get_catalog(psy_token: &str, session_id: &str, player_id: &str, category: &str) -> Result<serde_json::Value, String> {
    let body = serde_json::json!({
        "PlayerID": format!("Epic|{}|0", player_id),
        "Category": category
    });
    call_rpc("Microtransaction/GetCatalog v1", &body, Some(psy_token), Some(session_id)).await
}
