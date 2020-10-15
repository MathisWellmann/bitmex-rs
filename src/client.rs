use crate::error::{BitMEXError, BitMEXErrorResponse};
use crate::models::swagger::SwaggerApiDescription;
use crate::models::Request;
use crate::SWAGGER_URL;
use chrono::{Duration, Utc};
use derive_builder::Builder;
use fehler::{throw, throws};
use hex::encode as hexify;
use hyper::Method;
use log::{error, trace};
use reqwest::{Client, Response};
use ring::hmac;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{from_str, to_string, to_value};
use url::Url;

const EXPIRE_DURATION: i64 = 5;

#[derive(Clone, Builder)]
pub struct BitMEX {
    client: Client,
    #[builder(default)]
    credential: Option<(String, String)>,
    pub(crate) testnet: bool,
}

impl Default for BitMEX {
    fn default() -> Self {
        Self::new()
    }
}

impl BitMEX {
    pub fn new() -> Self {
        BitMEX {
            client: Client::new(),
            credential: None,
            testnet: false,
        }
    }

    pub fn with_credential(api_key: &str, api_secret: &str) -> Self {
        BitMEX {
            client: Client::new(),
            credential: Some((api_key.into(), api_secret.into())),
            testnet: false,
        }
    }

    /// if value is set to true, this client will call the testnet
    pub fn set_testnet(&mut self, value: bool) {
        self.testnet = value;
    }

    pub fn builder() -> BitMEXBuilder {
        BitMEXBuilder::default()
    }

    #[throws(failure::Error)]
    pub async fn request<R>(&self, req: R) -> R::Response
    where
        R: Request,
        R::Response: DeserializeOwned,
    {
        let rest_url: &str = match self.testnet {
            true => crate::consts::REST_URL_TESTNET,
            false => crate::consts::REST_URL_MAINNET,
        };
        let url = format!("{}{}", rest_url, R::ENDPOINT);
        let url = match R::METHOD {
            Method::GET | Method::DELETE => {
                if R::HAS_PAYLOAD {
                    Url::parse_with_params(&url, req.to_url_query())?
                } else {
                    Url::parse(&url)?
                }
            }
            _ => Url::parse(&url)?,
        };

        let body = match R::METHOD {
            Method::PUT | Method::POST => to_string(&req)?,
            _ => "".to_string(),
        };

        let mut builder = self.client.request(R::METHOD, url.clone());

        if R::SIGNED {
            let expires = (Utc::now() + Duration::seconds(EXPIRE_DURATION)).timestamp();
            let (key, signature) = self.signature(R::METHOD, expires, &url, &body)?;

            builder = builder
                .header("api-expires", expires)
                .header("api-key", key)
                .header("api-signature", signature);
        }

        let resp = builder
            .header("content-type", "application/json")
            .header("user-agent", "bitmex-rs")
            .body(body)
            .send()
            .await?;

        self.handle_response(resp).await?
    }

    #[throws(failure::Error)]
    fn check_key(&self) -> (&str, &str) {
        match self.credential.as_ref() {
            None => throw!(BitMEXError::NoApiKeySet),
            Some((k, s)) => (k.as_str(), s.as_str()),
        }
    }

    #[throws(failure::Error)]
    pub(crate) fn signature(
        &self,
        method: Method,
        expires: i64,
        url: &Url,
        body: &str,
    ) -> (&str, String) {
        let (key, secret) = self.check_key()?;
        // Signature: hex(HMAC_SHA256(apiSecret, verb + path + expires + data))
        let signed_key = hmac::Key::new(hmac::HMAC_SHA256, secret.as_bytes());
        let sign_message = match url.query() {
            Some(query) => format!(
                "{}{}?{}{}{}",
                method.as_str(),
                url.path(),
                query,
                expires,
                body
            ),
            None => format!("{}{}{}{}", method.as_str(), url.path(), expires, body),
        };
        trace!("Sign message {}", sign_message);
        let signature = hexify(hmac::sign(&signed_key, sign_message.as_bytes()));
        (key, signature)
    }

    #[throws(failure::Error)]
    async fn handle_response<T: DeserializeOwned>(&self, resp: Response) -> T {
        if resp.status().is_success() {
            let resp = resp.text().await?;
            match from_str::<T>(&resp) {
                Ok(resp) => resp,
                Err(e) => {
                    error!("Cannot deserialize '{}'", resp);
                    throw!(e);
                }
            }
        } else {
            let resp: BitMEXErrorResponse = resp.json().await?;
            throw!(BitMEXError::from(resp.error))
        }
    }

    #[throws(failure::Error)]
    pub async fn get_swagger(&self) -> SwaggerApiDescription {
        let resp: Response = self
            .client
            .get(SWAGGER_URL)
            .header("user-agent", "bitmex-rs")
            .header("content-type", "application/json")
            .send()
            .await?;
        self.handle_response(resp).await?
    }
}

trait ToUrlQuery: Serialize {
    fn to_url_query_string(&self) -> String {
        let vec = self.to_url_query();
        vec.into_iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect::<Vec<_>>()
            .join("&")
    }

    fn to_url_query(&self) -> Vec<(String, String)> {
        let v = to_value(self).unwrap();
        let v = v.as_object().unwrap();
        let mut vec = vec![];

        for (key, value) in v.into_iter() {
            if value.is_null() {
                continue;
            } else if value.is_string() {
                vec.push((key.clone(), value.as_str().unwrap().to_string()))
            } else {
                vec.push((key.clone(), to_string(value).unwrap()))
            }
        }
        vec
    }
}

impl<S: Serialize> ToUrlQuery for S {}

#[cfg(test)]
mod test {
    use hyper::Method;
    use url::Url;

    use super::BitMEX;
    use failure::Fallible;

    #[test]
    fn test_signature_get() -> Fallible<()> {
        let tr = BitMEX::with_credential(
            "LAqUlngMIQkIUjXMUreyu3qn",
            "chNOOS4KvNXR_Xq4k4c9qsfoKWvnDecLATCRlcBwyKDYnWgO",
        );
        let (_, sig) = tr.signature(
            Method::GET,
            1518064236,
            &Url::parse("http://a.com/api/v1/instrument")?,
            "",
        )?;
        assert_eq!(
            sig,
            "c7682d435d0cfe87c16098df34ef2eb5a549d4c5a3c2b1f0f77b8af73423bf00"
        );
        Ok(())
    }

    #[test]
    fn test_signature_get_param() -> Fallible<()> {
        let tr = BitMEX::with_credential(
            "LAqUlngMIQkIUjXMUreyu3qn",
            "chNOOS4KvNXR_Xq4k4c9qsfoKWvnDecLATCRlcBwyKDYnWgO",
        );
        let (_, sig) = tr.signature(
            Method::GET,
            1518064237,
            &Url::parse_with_params(
                "http://a.com/api/v1/instrument",
                &[("filter", r#"{"symbol": "XBTM15"}"#)],
            )?,
            "",
        )?;
        assert_eq!(
            sig,
            "e2f422547eecb5b3cb29ade2127e21b858b235b386bfa45e1c1756eb3383919f"
        );
        Ok(())
    }

    #[test]
    fn test_signature_post() -> Fallible<()> {
        let tr = BitMEX::with_credential(
            "LAqUlngMIQkIUjXMUreyu3qn",
            "chNOOS4KvNXR_Xq4k4c9qsfoKWvnDecLATCRlcBwyKDYnWgO",
        );
        let (_, sig) = tr.signature(
            Method::POST,
            1518064238,
            &Url::parse("http://a.com/api/v1/order")?,
            r#"{"symbol":"XBTM15","price":219.0,"clOrdID":"mm_bitmex_1a/oemUeQ4CAJZgP3fjHsA","orderQty":98}"#,
        )?;
        assert_eq!(
            sig,
            "1749cd2ccae4aa49048ae09f0b95110cee706e0944e6a14ad0b3a8cb45bd336b"
        );
        Ok(())
    }
}
