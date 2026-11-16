//! Looksie's shop: prices, checkout links, and licence keys with Polar.
//!
//! Keys are checked with Polar's public licence-key endpoints, which need no secret: a desktop app may call them.
//! Activating a key uses one of its 3 devices; removing it here frees that device again.
//!
//! Which Polar: a build made with LOOKSIE_POLAR_SANDBOX set talks to Polar's sandbox (test payments only).
//! Every other build uses the live store. Until the live store's ids are filled in below, its Buy buttons say the
//! checkout is being set up, and only Marcus's private test key works.

/// Shown on the plan screen. Change them here and nowhere else.
pub const PRICE_MONTHLY: &str = "$10";
pub const PRICE_YEARLY: &str = "$96";
pub const OFFER: &str = "First 100 Approvers: 40% off the yearly plan, for life. Use code APPROVERS at checkout.";

/// How many launch-offer places are left. A GitHub Action reads the count from Polar every hour and publishes it here,
/// so no store key is ever inside the app. The offer is hidden at 0, and when this cannot be read.
pub const OFFER_URL: &str = "https://looksie.dev/offer.json";

pub struct Polar {
    pub api: &'static str,
    pub organization: &'static str,
    /// Where customers manage their subscription and free up a device.
    pub portal: &'static str,
    pub checkout_monthly: &'static str,
    pub checkout_yearly: &'static str,
}

const SANDBOX: bool = option_env!("LOOKSIE_POLAR_SANDBOX").is_some();

pub const POLAR: Polar = if SANDBOX {
    Polar {
        api: "https://sandbox-api.polar.sh",
        organization: "2cacb3e4-64ea-4468-8c93-c73524601daa",
        portal: "https://sandbox.polar.sh/looksie/portal",
        checkout_monthly: "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_ExxJOnDolaf4XrrzLzHqBYbM0ggxmbp0jCOxE2RNPFZ/redirect",
        checkout_yearly: "https://sandbox-api.polar.sh/v1/checkout-links/polar_cl_ZTvW48BxkKbSJ7vJvRGqpb5QE3X358U8yiV7s02RAUf/redirect",
    }
} else {
    // The live store on polar.sh.
    Polar {
        api: "https://api.polar.sh",
        organization: "e6f03922-d4be-4da4-a1d7-1e3c7c1244ed",
        portal: "https://polar.sh/looksie/portal",
        checkout_monthly: "https://buy.polar.sh/polar_cl_YlQv8mmfOQQ45ouJLJKEi2zf5QNsQ4VArVfK54A3g4P",
        checkout_yearly: "https://buy.polar.sh/polar_cl_NonHyAQubtQiT2fVN4RKAN7WFb4WPPHnCmyGM2jNFZY",
    }
};

/// SHA-256 of Marcus's private test key, which works without the store. The app holds only this fingerprint.
const TEST_KEY_SHA256: &str = "c74d8090390d0c71ffdced4a6261c0f5a60fb743467663e9766843efc4665a0b";

pub fn is_known_key(key: &str) -> bool {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(key.trim().as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    hex == TEST_KEY_SHA256
}

/// What the store said about a key.
pub enum Check {
    Good,
    /// Unknown, revoked (subscription ended or cancelled), or this computer's activation was removed.
    Gone,
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| e.to_string())
}

fn ready() -> Result<(), String> {
    if POLAR.organization.is_empty() {
        Err("The store is not open yet, so this key cannot be checked. Try again after launch.".into())
    } else {
        Ok(())
    }
}

/// A short name for this computer, shown to the customer in Polar's portal next to the activation.
pub fn device_label() -> String {
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).ok().filter(|n| !n.trim().is_empty()).or_else(|| {
        crate::quiet("hostname").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).filter(|n| !n.is_empty())
    });
    let os = match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "Mac",
        other => other,
    };
    format!("{} ({os})", name.unwrap_or_else(|| "This computer".into())).chars().take(80).collect()
}

/// Uses one of the key's devices for this computer. Returns the activation's id.
pub async fn activate(key: &str) -> Result<String, String> {
    ready()?;
    let res = client()?
        .post(format!("{}/v1/customer-portal/license-keys/activate", POLAR.api))
        .json(&serde_json::json!({ "key": key, "organization_id": POLAR.organization, "label": device_label() }))
        .send()
        .await
        .map_err(|_| "Could not reach the store. Check your internet connection and try again.".to_string())?;
    match res.status().as_u16() {
        200 | 201 => {
            let body: serde_json::Value = res.json().await.map_err(|_| "The store sent something unexpected.".to_string())?;
            body.get("id").and_then(|v| v.as_str()).map(str::to_string).ok_or_else(|| "The store sent something unexpected.".to_string())
        }
        404 | 422 => Err("The store does not recognise that key. Check you copied all of it.".into()),
        403 => Err(format!(
            "That key cannot be used here. It is already on 3 devices, or its subscription has ended. You can free a device at {}",
            POLAR.portal
        )),
        s => Err(format!("The store could not check the key just now ({s}). Try again in a minute.")),
    }
}

/// Asks the store whether this computer's activation of the key is still good. Err means the store could not be reached.
pub async fn validate(key: &str, activation: &str) -> Result<Check, String> {
    ready()?;
    let res = client()?
        .post(format!("{}/v1/customer-portal/license-keys/validate", POLAR.api))
        .json(&serde_json::json!({ "key": key, "organization_id": POLAR.organization, "activation_id": activation }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    match res.status().as_u16() {
        200 => {
            let body: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
            Ok(if body.get("status").and_then(|v| v.as_str()) == Some("granted") { Check::Good } else { Check::Gone })
        }
        404 => Ok(Check::Gone),
        s => Err(format!("store answered {s}")),
    }
}

/// Frees this computer's device. Best effort: if the store cannot be reached, the customer can still free it in the portal.
pub async fn deactivate(key: &str, activation: &str) {
    if ready().is_err() {
        return;
    }
    if let Ok(c) = client() {
        let _ = c
            .post(format!("{}/v1/customer-portal/license-keys/deactivate", POLAR.api))
            .json(&serde_json::json!({ "key": key, "organization_id": POLAR.organization, "activation_id": activation }))
            .send()
            .await;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_made_up_key_is_not_the_test_key() {
        assert!(!super::is_known_key("ABCDEFGHIJKLMNOPQRSTUVWX"));
        assert!(!super::is_known_key("LOOKSIE-DEV-00000000000000000000000000000000"));
    }

    #[test]
    fn public_builds_never_point_at_the_sandbox() {
        if !super::SANDBOX {
            assert!(!super::POLAR.api.contains("sandbox"));
            assert!(!super::POLAR.checkout_yearly.contains("sandbox"));
        }
    }
}
