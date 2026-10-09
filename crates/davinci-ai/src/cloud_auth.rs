//! Request-time credentials for cloud providers whose ambient credentials are
//! not a bearer key: AWS SigV4 signing for Amazon Bedrock and Application
//! Default Credentials access tokens for Google Vertex.
//!
//! Auth resolution stays offline; it only reports a provider usable when the
//! credential kind is one this module can turn into request headers.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::ResolvedAuth;
use crate::catalog::Model;

/// Headers the provider request needs beyond the resolved auth: a SigV4
/// signature for Bedrock or an ADC access token for Vertex. Empty when the
/// resolved auth already authenticates the request.
pub(crate) fn request_auth_headers(
    model: &Model,
    auth: &ResolvedAuth,
    url: &str,
    body: &Value,
) -> Result<Vec<(String, String)>, String> {
    let has_authorization = auth
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"));
    if has_authorization || auth.api_key.is_some() {
        return Ok(Vec::new());
    }
    match model.api.as_str() {
        "bedrock-converse-stream" => {
            // Ambient AWS credentials (a session token travels in a header)
            // go only to AWS endpoints, never to a custom base URL.
            require_host(
                url,
                &[".amazonaws.com", ".amazonaws.com.cn"],
                "Amazon Bedrock",
            )?;
            let credentials = aws_credentials(&process_env())?.ok_or(
                "Amazon Bedrock needs AWS_BEARER_TOKEN_BEDROCK, AWS access keys, or an AWS_PROFILE with static keys",
            )?;
            sign_bedrock(&credentials, url, &body.to_string(), SystemTime::now())
        }
        "google-vertex" => {
            require_host(url, &[".googleapis.com"], "Google Vertex")?;
            let adc = vertex_adc_path(&process_env());
            let token = vertex_access_token(&adc)?;
            Ok(vec![("Authorization".into(), format!("Bearer {token}"))])
        }
        _ => Ok(Vec::new()),
    }
}

/// Ambient cloud credentials are attached only for HTTPS requests to the
/// provider's own domains.
fn require_host(url: &str, suffixes: &[&str], provider: &str) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("{provider} URL: {error}"))?;
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if parsed.scheme() == "https" && suffixes.iter().any(|suffix| host.ends_with(suffix)) {
        Ok(())
    } else {
        Err(format!(
            "{provider} ambient credentials are only sent to its HTTPS endpoints; {host} needs an explicit API key"
        ))
    }
}

/// Token endpoints receive refresh tokens or signed assertions: HTTPS only,
/// except loopback (local test servers).
fn require_secure_token_uri(token_uri: &str) -> Result<(), String> {
    let parsed = url::Url::parse(token_uri)
        .map_err(|_| "Google credentials have an invalid token_uri".to_string())?;
    let loopback = matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    if parsed.scheme() == "https" || (parsed.scheme() == "http" && loopback) {
        Ok(())
    } else {
        Err("Google credentials token_uri must use https".into())
    }
}

fn process_env() -> HashMap<String, String> {
    std::env::vars().collect()
}

fn env_value(env: &HashMap<String, String>, name: &str) -> Option<String> {
    env.get(name).cloned().filter(|value| !value.is_empty())
}

// ---------------------------------------------------------------------------
// AWS
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AwsCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: Option<String>,
}

/// Static AWS keys from the environment, or from the shared credentials file
/// for `AWS_PROFILE`. Role-based sources (ECS task role, web identity) need
/// an STS exchange and are not supported here.
pub(crate) fn aws_credentials(
    env: &HashMap<String, String>,
) -> Result<Option<AwsCredentials>, String> {
    if let (Some(access_key_id), Some(secret_access_key)) = (
        env_value(env, "AWS_ACCESS_KEY_ID"),
        env_value(env, "AWS_SECRET_ACCESS_KEY"),
    ) {
        return Ok(Some(AwsCredentials {
            access_key_id,
            secret_access_key,
            session_token: env_value(env, "AWS_SESSION_TOKEN"),
        }));
    }
    let Some(profile) = env_value(env, "AWS_PROFILE") else {
        return Ok(None);
    };
    let path = env_value(env, "AWS_SHARED_CREDENTIALS_FILE")
        .map(PathBuf::from)
        .or_else(|| crate::auth::home_dir().map(|home| home.join(".aws").join("credentials")));
    let Some(text) = path.and_then(|path| std::fs::read_to_string(path).ok()) else {
        return Ok(None);
    };
    Ok(profile_credentials(&text, &profile))
}

fn profile_credentials(ini: &str, profile: &str) -> Option<AwsCredentials> {
    let mut in_profile = false;
    let mut values = HashMap::new();
    for line in ini.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.starts_with(';') || line.is_empty() {
            continue;
        }
        if let Some(section) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let section = section.trim();
            in_profile = section == profile || section == format!("profile {profile}");
            continue;
        }
        if in_profile {
            if let Some((key, value)) = line.split_once('=') {
                values.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
            }
        }
    }
    Some(AwsCredentials {
        access_key_id: values.remove("aws_access_key_id")?,
        secret_access_key: values.remove("aws_secret_access_key")?,
        session_token: values.remove("aws_session_token"),
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA256, key);
    ring::hmac::sign(&key, data.as_bytes()).as_ref().to_vec()
}

/// SigV4 URI encoding: keep RFC 3986 unreserved characters, escape the rest.
fn aws_encode(segment: &str) -> String {
    let mut out = String::new();
    for byte in segment.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `YYYYMMDDTHHMMSSZ` for a Unix timestamp, without a date library.
fn amz_date(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

pub(crate) struct SigningRequest<'a> {
    pub method: &'a str,
    /// Path as sent on the wire (already percent-encoded once).
    pub path: &'a str,
    pub query: &'a str,
    pub headers: &'a [(String, String)],
    pub payload: &'a str,
    pub region: &'a str,
    pub service: &'a str,
}

/// AWS Signature Version 4. Returns the `Authorization` header value.
pub(crate) fn sigv4_authorization(
    credentials: &AwsCredentials,
    request: &SigningRequest<'_>,
    amz_date: &str,
    double_encode_path: bool,
) -> String {
    let date = &amz_date[..8];
    let canonical_uri = if double_encode_path {
        request
            .path
            .split('/')
            .map(aws_encode)
            .collect::<Vec<_>>()
            .join("/")
    } else {
        request.path.to_string()
    };
    let mut query: Vec<(String, String)> = url::form_urlencoded::parse(request.query.as_bytes())
        .map(|(key, value)| (aws_encode(&key), aws_encode(&value)))
        .collect();
    query.sort();
    let canonical_query = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut headers: Vec<(String, String)> = request
        .headers
        .iter()
        .map(|(name, value)| (name.to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    headers.sort();
    let canonical_headers: String = headers
        .iter()
        .map(|(name, value)| format!("{name}:{value}\n"))
        .collect();
    let signed_headers = headers
        .iter()
        .map(|(name, _)| name.as_str())
        .collect::<Vec<_>>()
        .join(";");
    let canonical_request = format!(
        "{}\n{canonical_uri}\n{canonical_query}\n{canonical_headers}\n{signed_headers}\n{}",
        request.method,
        hex(&Sha256::digest(request.payload.as_bytes()))
    );
    let scope = format!("{date}/{}/{}/aws4_request", request.region, request.service);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}",
        hex(&Sha256::digest(canonical_request.as_bytes()))
    );
    let mut key = hmac(
        format!("AWS4{}", credentials.secret_access_key).as_bytes(),
        date,
    );
    for part in [request.region, request.service, "aws4_request"] {
        key = hmac(&key, part);
    }
    let signature = hex(&hmac(&key, &string_to_sign));
    format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_headers}, Signature={signature}",
        credentials.access_key_id
    )
}

fn bedrock_region(host: &str) -> String {
    let labels: Vec<&str> = host.split('.').collect();
    if labels.len() >= 4 && labels[0].starts_with("bedrock-runtime") {
        return labels[1].to_string();
    }
    std::env::var("AWS_REGION")
        .or_else(|_| std::env::var("AWS_DEFAULT_REGION"))
        .ok()
        .filter(|region| !region.is_empty())
        .unwrap_or_else(|| "us-east-1".into())
}

fn sign_bedrock(
    credentials: &AwsCredentials,
    url: &str,
    payload: &str,
    now: SystemTime,
) -> Result<Vec<(String, String)>, String> {
    let parsed = url::Url::parse(url).map_err(|error| format!("Bedrock URL: {error}"))?;
    let host = match (parsed.host_str(), parsed.port()) {
        (Some(host), Some(port)) => format!("{host}:{port}"),
        (Some(host), None) => host.to_string(),
        (None, _) => return Err("Bedrock URL: missing host".into()),
    };
    let amz_date = amz_date(now);
    let payload_hash = hex(&Sha256::digest(payload.as_bytes()));
    let mut signed = vec![
        ("host".to_string(), host.clone()),
        ("x-amz-content-sha256".to_string(), payload_hash.clone()),
        ("x-amz-date".to_string(), amz_date.clone()),
    ];
    if let Some(token) = &credentials.session_token {
        signed.push(("x-amz-security-token".to_string(), token.clone()));
    }
    let region = bedrock_region(parsed.host_str().unwrap_or_default());
    let authorization = sigv4_authorization(
        credentials,
        &SigningRequest {
            method: "POST",
            path: parsed.path(),
            query: parsed.query().unwrap_or(""),
            headers: &signed,
            payload,
            region: &region,
            service: "bedrock",
        },
        &amz_date,
        true,
    );
    let mut headers: Vec<(String, String)> = signed
        .into_iter()
        .filter(|(name, _)| name != "host")
        .collect();
    headers.push(("Authorization".into(), authorization));
    Ok(headers)
}

// ---------------------------------------------------------------------------
// Google Application Default Credentials
// ---------------------------------------------------------------------------

const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const CLOUD_PLATFORM_SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";

pub(crate) fn vertex_adc_path(env: &HashMap<String, String>) -> PathBuf {
    let raw = env_value(env, "GOOGLE_APPLICATION_CREDENTIALS")
        .unwrap_or_else(|| "~/.config/gcloud/application_default_credentials.json".into());
    match raw.strip_prefix("~/") {
        Some(rest) => crate::auth::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(&raw)),
        None => PathBuf::from(raw),
    }
}

/// Whether the ADC file is a kind this module can exchange for a token.
pub(crate) fn supported_adc(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|adc| {
            matches!(
                adc.get("type").and_then(Value::as_str),
                Some("authorized_user" | "service_account")
            )
        })
}

static TOKEN_CACHE: Mutex<Option<HashMap<PathBuf, (String, Instant)>>> = Mutex::new(None);

fn vertex_access_token(path: &Path) -> Result<String, String> {
    if let Some((token, valid_until)) = TOKEN_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(|cache| cache.get(path).cloned())
    {
        if Instant::now() < valid_until {
            return Ok(token);
        }
    }
    let text = std::fs::read_to_string(path).map_err(|error| {
        format!(
            "Google application default credentials unreadable ({}): {error}",
            path.display()
        )
    })?;
    let adc: Value = serde_json::from_str(&text)
        .map_err(|_| "Google application default credentials are not valid JSON".to_string())?;
    let (token, lifetime) = exchange_adc(&adc, SystemTime::now())?;
    // Renew a minute early.
    let valid_until = Instant::now() + lifetime.saturating_sub(Duration::from_secs(60));
    TOKEN_CACHE
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get_or_insert_with(HashMap::new)
        .insert(path.to_path_buf(), (token.clone(), valid_until));
    Ok(token)
}

fn base64url(bytes: &[u8]) -> String {
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

/// RS256 JWT bearer assertion for a service account (RFC 7523).
pub(crate) fn service_account_assertion(
    adc: &Value,
    token_uri: &str,
    now: SystemTime,
) -> Result<String, String> {
    let email = adc
        .get("client_email")
        .and_then(Value::as_str)
        .ok_or("service account credentials have no client_email")?;
    let pem = adc
        .get("private_key")
        .and_then(Value::as_str)
        .ok_or("service account credentials have no private_key")?;
    let der_b64: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect::<Vec<_>>()
        .concat();
    let der = base64::Engine::decode(&base64::engine::general_purpose::STANDARD, der_b64.trim())
        .map_err(|_| "service account private_key is not valid PEM".to_string())?;
    let key = ring::signature::RsaKeyPair::from_pkcs8(&der)
        .map_err(|_| "service account private_key is not a PKCS#8 RSA key".to_string())?;
    let iat = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
    let header = base64url(br#"{"alg":"RS256","typ":"JWT"}"#);
    let claims = base64url(
        serde_json::json!({
            "iss": email,
            "scope": CLOUD_PLATFORM_SCOPE,
            "aud": token_uri,
            "iat": iat,
            "exp": iat + 3600,
        })
        .to_string()
        .as_bytes(),
    );
    let message = format!("{header}.{claims}");
    let mut signature = vec![0; key.public().modulus_len()];
    key.sign(
        &ring::signature::RSA_PKCS1_SHA256,
        &ring::rand::SystemRandom::new(),
        message.as_bytes(),
        &mut signature,
    )
    .map_err(|_| "unable to sign the service account assertion".to_string())?;
    Ok(format!("{message}.{}", base64url(&signature)))
}

fn exchange_adc(adc: &Value, now: SystemTime) -> Result<(String, Duration), String> {
    let field = |name: &str| adc.get(name).and_then(Value::as_str);
    let token_uri = field("token_uri").unwrap_or(GOOGLE_TOKEN_URL).to_string();
    require_secure_token_uri(&token_uri)?;
    let form: Vec<(&str, String)> = match field("type") {
        Some("authorized_user") => vec![
            ("grant_type", "refresh_token".into()),
            (
                "client_id",
                field("client_id")
                    .ok_or("authorized_user credentials have no client_id")?
                    .into(),
            ),
            (
                "client_secret",
                field("client_secret")
                    .ok_or("authorized_user credentials have no client_secret")?
                    .into(),
            ),
            (
                "refresh_token",
                field("refresh_token")
                    .ok_or("authorized_user credentials have no refresh_token")?
                    .into(),
            ),
        ],
        Some("service_account") => vec![
            (
                "grant_type",
                "urn:ietf:params:oauth:grant-type:jwt-bearer".into(),
            ),
            (
                "assertion",
                service_account_assertion(adc, &token_uri, now)?,
            ),
        ],
        other => {
            return Err(format!(
                "Google application default credentials of type {other:?} are not supported; use `gcloud auth application-default login` or a service account key"
            ))
        }
    };
    let body = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(form.iter().map(|(key, value)| (*key, value.as_str())))
        .finish();
    let response = crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT)
        .post(&token_uri)
        .set("content-type", "application/x-www-form-urlencoded")
        .set("accept", "application/json")
        .send_string(&body)
        .map_err(|error| match error {
            ureq::Error::Status(status, _) => {
                format!("Google token exchange was rejected (status {status})")
            }
            other => format!("Google token exchange failed: {other}"),
        })?;
    let reply: Value = serde_json::from_str(
        &response
            .into_string()
            .map_err(|error| format!("Google token response unreadable: {error}"))?,
    )
    .map_err(|_| "Google token response is not JSON".to_string())?;
    let token = reply
        .get("access_token")
        .and_then(Value::as_str)
        .ok_or("Google token response has no access_token")?;
    let lifetime = reply
        .get("expires_in")
        .and_then(Value::as_u64)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(3600));
    Ok((token.to_string(), lifetime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sigv4_matches_the_aws_documented_example() {
        // docs.aws.amazon.com/IAM/latest/UserGuide/create-signed-request.html
        // (IAM ListUsers, the long-standing published example).
        let credentials = AwsCredentials {
            access_key_id: "AKIDEXAMPLE".into(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let headers = vec![
            (
                "content-type".to_string(),
                "application/x-www-form-urlencoded; charset=utf-8".to_string(),
            ),
            ("host".to_string(), "iam.amazonaws.com".to_string()),
            ("x-amz-date".to_string(), "20150830T123600Z".to_string()),
        ];
        let authorization = sigv4_authorization(
            &credentials,
            &SigningRequest {
                method: "GET",
                path: "/",
                query: "Action=ListUsers&Version=2010-05-08",
                headers: &headers,
                payload: "",
                region: "us-east-1",
                service: "iam",
            },
            "20150830T123600Z",
            false,
        );
        assert_eq!(
            authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }

    #[test]
    fn ambient_credentials_only_go_to_provider_https_hosts() {
        let suffixes = [".amazonaws.com", ".amazonaws.com.cn"];
        assert!(require_host(
            "https://bedrock-runtime.us-east-1.amazonaws.com/model/x/converse",
            &suffixes,
            "Amazon Bedrock"
        )
        .is_ok());
        for url in [
            "https://proxy.example.com/model/x/converse",
            "https://amazonaws.com.attacker.test/model/x/converse",
            "http://bedrock-runtime.us-east-1.amazonaws.com/model/x/converse",
        ] {
            assert!(
                require_host(url, &suffixes, "Amazon Bedrock").is_err(),
                "{url}"
            );
        }
        assert!(require_secure_token_uri("https://oauth2.googleapis.com/token").is_ok());
        assert!(require_secure_token_uri("http://127.0.0.1:9/token").is_ok());
        assert!(require_secure_token_uri("http://tokens.example.com/token").is_err());
    }

    #[test]
    fn amz_date_formats_utc() {
        let time = UNIX_EPOCH + Duration::from_secs(1_440_938_160);
        assert_eq!(amz_date(time), "20150830T123600Z");
        assert_eq!(amz_date(UNIX_EPOCH), "19700101T000000Z");
    }

    #[test]
    fn bedrock_requests_are_signed_with_region_and_session_token() {
        let credentials = AwsCredentials {
            access_key_id: "AKID".into(),
            secret_access_key: "secret".into(),
            session_token: Some("session".into()),
        };
        let headers = sign_bedrock(
            &credentials,
            "https://bedrock-runtime.eu-west-1.amazonaws.com/model/anthropic.claude-v2%3A1/converse",
            "{}",
            UNIX_EPOCH + Duration::from_secs(1_440_938_160),
        )
        .unwrap();
        let get = |name: &str| {
            headers
                .iter()
                .find(|(key, _)| key.eq_ignore_ascii_case(name))
                .map(|(_, value)| value.as_str())
        };
        let authorization = get("authorization").unwrap();
        assert!(authorization.contains("/20150830/eu-west-1/bedrock/aws4_request"));
        assert!(authorization
            .contains("SignedHeaders=host;x-amz-content-sha256;x-amz-date;x-amz-security-token"));
        assert_eq!(get("x-amz-security-token"), Some("session"));
        assert_eq!(get("x-amz-date"), Some("20150830T123600Z"));
    }

    #[test]
    fn profile_credentials_are_read_from_the_shared_file() {
        let ini = "[default]\naws_access_key_id = A\naws_secret_access_key = B\n\n[work]\naws_access_key_id=C\naws_secret_access_key=D\naws_session_token=E\n[role]\nrole_arn = arn:aws:iam::1:role/x\n";
        assert_eq!(
            profile_credentials(ini, "work"),
            Some(AwsCredentials {
                access_key_id: "C".into(),
                secret_access_key: "D".into(),
                session_token: Some("E".into()),
            })
        );
        assert_eq!(profile_credentials(ini, "role"), None);
    }

    /// A fresh RSA key pair (PKCS#8 PEM, PKCS#1 public DER) from the
    /// `openssl` CLI, so no private key is committed. `None` without openssl.
    fn throwaway_rsa_key() -> Option<(String, Vec<u8>)> {
        let dir = tempfile::tempdir().ok()?;
        let key = dir.path().join("key.pem");
        let public = dir.path().join("pub.der");
        let run = |args: &[&std::ffi::OsStr]| {
            std::process::Command::new("openssl")
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|status| status.success())
        };
        let generated = run(&[
            "genpkey".as_ref(),
            "-algorithm".as_ref(),
            "RSA".as_ref(),
            "-pkeyopt".as_ref(),
            "rsa_keygen_bits:2048".as_ref(),
            "-out".as_ref(),
            key.as_os_str(),
        ]) && run(&[
            "rsa".as_ref(),
            "-in".as_ref(),
            key.as_os_str(),
            "-RSAPublicKey_out".as_ref(),
            "-outform".as_ref(),
            "DER".as_ref(),
            "-out".as_ref(),
            public.as_os_str(),
        ]);
        if !generated {
            return None;
        }
        Some((
            std::fs::read_to_string(&key).ok()?,
            std::fs::read(&public).ok()?,
        ))
    }

    #[test]
    fn service_account_assertion_is_a_verifiable_rs256_jwt() {
        let Some((private_key, public_der)) = throwaway_rsa_key() else {
            eprintln!("skipping: openssl is not available to generate a test key");
            return;
        };
        let adc = serde_json::json!({
            "type": "service_account",
            "client_email": "svc@project.iam.gserviceaccount.com",
            "private_key": private_key,
        });
        let jwt = service_account_assertion(&adc, GOOGLE_TOKEN_URL, UNIX_EPOCH).unwrap();
        let parts: Vec<&str> = jwt.split('.').collect();
        assert_eq!(parts.len(), 3);
        let decode = |part: &str| {
            base64::Engine::decode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, part).unwrap()
        };
        let claims: Value = serde_json::from_slice(&decode(parts[1])).unwrap();
        assert_eq!(claims["iss"], "svc@project.iam.gserviceaccount.com");
        assert_eq!(claims["aud"], GOOGLE_TOKEN_URL);
        let public = ring::signature::UnparsedPublicKey::new(
            &ring::signature::RSA_PKCS1_2048_8192_SHA256,
            public_der.as_slice(),
        );
        public
            .verify(
                format!("{}.{}", parts[0], parts[1]).as_bytes(),
                &decode(parts[2]),
            )
            .unwrap();
    }

    #[test]
    fn authorized_user_adc_is_exchanged_for_a_bearer_token() {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let token_uri = format!("http://{}/token", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let reply = r#"{"access_token":"ya29.fixture","expires_in":3599}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{reply}",
                reply.len()
            )
            .unwrap();
            String::from_utf8(body).unwrap()
        });
        let adc = serde_json::json!({
            "type": "authorized_user",
            "client_id": "cid",
            "client_secret": "csecret",
            "refresh_token": "rtoken",
            "token_uri": token_uri,
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adc.json");
        std::fs::write(&path, adc.to_string()).unwrap();
        assert!(supported_adc(&path));
        assert_eq!(vertex_access_token(&path).unwrap(), "ya29.fixture");
        // Cached: a second call does not hit the (now closed) server.
        assert_eq!(vertex_access_token(&path).unwrap(), "ya29.fixture");
        let form = server.join().unwrap();
        assert!(form.contains("grant_type=refresh_token"), "{form}");
        assert!(form.contains("refresh_token=rtoken"), "{form}");
    }
}
