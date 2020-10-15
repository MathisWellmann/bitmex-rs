use super::Topic;
use crate::consts::WS_URL_MAINNET;
use crate::BitMEX;
use fehler::throws;
use hyper::Method;
use serde_derive::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", content = "args")]
#[serde(rename_all = "camelCase")]
pub enum Command {
    Subscribe(Vec<Topic>),
    Unsubscribe(Vec<Topic>),
    #[serde(rename = "authKeyExpires")]
    Authenticate(String, i64, String), // ApiKey, Expires, Signature
    CancelAllAfter(i64),
    Ping,
}

impl Command {
    #[throws(failure::Error)]
    pub fn authenticate(bm: &BitMEX, expires: i64) -> Command {
        let (key, sig) = bm.signature(Method::GET, expires, &Url::parse(WS_URL_MAINNET)?, "")?;
        Command::Authenticate(key.to_string(), expires, sig)
    }
}
