use hmac::{Hmac, Mac};
use sha2::Sha256;
use serde::{Deserialize, Serialize};
use base64::{Engine as _, engine::general_purpose};
use std::collections::HashSet;

type HmacSha256 = Hmac<Sha256>;

const PSYNET_RPC_URL: &str = "https://api.rlpp.psynet.gg/rpc/";

pub fn get_psysig(body: &str) -> String {
    let key_hex = std::env::var("PSYNET_REQUEST_KEY").expect("PSYNET_REQUEST_KEY must be set");
    let key = hex::decode(key_hex).unwrap();
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

pub const EPIC_LAUNCHER_CLIENT_ID: &str = "34a02cf8f4414e29b15921876da36f9a";

pub struct EpicAuth {
    pub client_id: String,
}

impl EpicAuth {
    pub fn new(client_id: &str) -> Self {
        Self { client_id: client_id.to_string() }
    }

    pub async fn exchange_code(&self, code: &str) -> Result<serde_json::Value, String> {
        let client = reqwest::Client::new();
        let body = [
            ("grant_type", "exchange_code"),
            ("exchange_code", code),
            ("client_id", &self.client_id),
            ("token_type", "eg1"),
        ];

        let resp = client.post("https://account-public-service-prod.ol.epicgames.com/account/api/oauth/token")
            .form(&body)
            .send()
            .await
            .map_err(|e| e.to_string())?;

        if resp.status() != 200 {
            return Err(format!("Epic Auth failed: {} - {}", resp.status(), resp.text().await.unwrap_or_default()));
        }

        resp.json().await.map_err(|e| e.to_string())
    }
}

pub struct PsynetClient {
    pub auth_ticket: String,
    pub session_id: Option<String>,
    pub psy_token: Option<String>,
    pub player_id: Option<String>,
}

impl PsynetClient {
    pub fn new(auth_ticket: String) -> Self {
        Self {
            auth_ticket,
            session_id: None,
            psy_token: None,
            player_id: None,
        }
    }

    pub async fn call_rpc<T: serde::de::DeserializeOwned>(
        &self,
        service: &str,
        body: &serde_json::Value,
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

        if let Some(ref token) = self.psy_token {
            headers.insert("PsyToken", token.parse().unwrap());
        }
        if let Some(ref id) = self.session_id {
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

    pub async fn login(&mut self, epic_account_id: &str) -> Result<(), String> {
        let body = serde_json::json!({
            "Platform": "Epic",
            "PlayerName": epic_account_id,
            "PlayerID": epic_account_id,
            "Language": "INT",
            "AuthTicket": self.auth_ticket,
            "FeatureSet": "PrimeUpdate55_1",
            "Device": "PC",
            "EpicAuthTicket": self.auth_ticket,
            "EpicAccountID": epic_account_id
        });

        let res: serde_json::Value = self.call_rpc("Auth/Login v4", &body).await?;
        self.psy_token = res["PsyToken"].as_str().map(|s| s.to_string());
        self.session_id = res["SessionID"].as_str().map(|s| s.to_string());
        self.player_id = Some(epic_account_id.to_string());
        Ok(())
    }

    pub async fn get_catalog(&self, category: &str) -> Result<serde_json::Value, String> {
        let player_id = self.player_id.as_ref().ok_or("Not logged in")?;
        let body = serde_json::json!({
            "PlayerID": format!("Epic|{}|0", player_id),
            "Category": category
        });
        self.call_rpc("Microtransaction/GetCatalog v1", &body).await
    }

    pub async fn get_all_products(&self) -> Result<Vec<serde_json::Value>, String> {
        let categories = vec!["StarterPack", "Shop", "Blueprint", "TradeIn"];
        let mut all_products = Vec::new();
        let mut seen_ids = HashSet::new();

        for cat in categories {
            match self.get_catalog(cat).await {
                Ok(res) => {
                    if let Some(products) = res["Products"].as_array() {
                        for p in products {
                            if let Some(pid) = p["ProductID"].as_i64() {
                                if seen_ids.insert(pid) {
                                    all_products.push(p.clone());
                                }
                            }
                        }
                    }
                }
                Err(e) => println!("Warning: Failed to fetch category {}: {}", cat, e),
            }
        }
    pub async fn get_player_products(&self, updated_timestamp: i64) -> Result<Vec<serde_json::Value>, String> {
        let player_id = self.player_id.as_ref().ok_or("Not logged in")?;
        let body = serde_json::json!({
            "PlayerID": format!("Epic|{}|0", player_id),
            "UpdatedTimestamp": updated_timestamp.to_string()
        });
        let res: serde_json::Value = self.call_rpc("Products/GetPlayerProducts v2", &body).await?;
        Ok(res["ProductData"].as_array().cloned().unwrap_or_default())
    }

    pub async fn get_container_drop_table(&self) -> Result<Vec<serde_json::Value>, String> {
        let body = serde_json::json!({});
        let res: serde_json::Value = self.call_rpc("Products/GetContainerDropTable v2", &body).await?;
        Ok(res["ContainerDrops"].as_array().cloned().unwrap_or_default())
    }

    pub async fn unlock_container(&self, instance_ids: Vec<String>) -> Result<Vec<serde_json::Value>, String> {
        let player_id = self.player_id.as_ref().ok_or("Not logged in")?;
        let body = serde_json::json!({
            "PlayerID": format!("Epic|{}|0", player_id),
            "InstanceIDs": instance_ids,
            "KeyInstanceIDs": []
        });
        let res: serde_json::Value = self.call_rpc("Products/UnlockContainer v2", &body).await?;
        Ok(res["Drops"].as_array().cloned().unwrap_or_default())
    }

    pub async fn trade_in(&self, product_instances: Vec<String>) -> Result<Vec<serde_json::Value>, String> {
        let player_id = self.player_id.as_ref().ok_or("Not logged in")?;
        let body = serde_json::json!({
            "PlayerID": format!("Epic|{}|0", player_id),
            "ProductInstances": product_instances
        });
        let res: serde_json::Value = self.call_rpc("Products/TradeIn v2", &body).await?;
        Ok(res["Drops"].as_array().cloned().unwrap_or_default())
    }

    pub async fn get_cross_entitlement_status(&self) -> Result<serde_json::Value, String> {
        let body = serde_json::json!({});
        self.call_rpc("Products/CrossEntitlement/GetProductStatus v1", &body).await
    }
}
