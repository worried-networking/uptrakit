use crate::client::ApiClient;
use crate::config::{
    Config, Credentials, load_config, load_credentials, save_config, save_credentials,
};
use crate::error::{CliError, Result};

/// Interactive login flow.
///
/// 1. Prompt for server URL (if not stored/provided)
/// 2. Prompt for email and password
/// 3. POST /api/v1/auth/login -> get JWT
/// 4. Use JWT to POST /api/v1/auth/api-tokens -> get API token
/// 5. Store server URL + API token locally
pub async fn login(server_override: Option<&str>) -> Result<()> {
    // Determine server URL
    let config = load_config()?;
    let server = if let Some(s) = server_override {
        s.to_string()
    } else if let Some(s) = &config.server {
        let input = prompt(&format!("Server URL [{}]: ", s))?;
        if input.is_empty() { s.clone() } else { input }
    } else {
        prompt("Server URL: ")?
    };

    if server.is_empty() {
        return Err(CliError::Other("Server URL is required".into()));
    }

    // Prompt for credentials
    let email = prompt("Email: ")?;
    eprint!("Password: ");
    let password = rpassword::read_password()
        .map_err(|e| CliError::Other(format!("Failed to read password: {e}")))?;

    // Login via password
    let client = ApiClient::new(&server, None)?;
    let login_body = serde_json::json!({
        "email": email,
        "password": password,
    });

    let (status, body) = client
        .request("POST", "/api/v1/auth/login", Some(login_body))
        .await?;

    if status != 200 {
        let msg = body.as_str().unwrap_or("Login failed").to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    let access_token = body["access_token"]
        .as_str()
        .ok_or_else(|| CliError::Other("No access_token in response".into()))?;

    // Create API token using the JWT
    let jwt_client = ApiClient::with_token(&server, access_token)?;
    let host = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    let date = chrono_date();
    let token_name = format!("cli-{host}-{date}");

    let create_body = serde_json::json!({ "name": token_name });
    let (status, body) = jwt_client
        .request("POST", "/api/v1/auth/api-tokens", Some(create_body))
        .await?;

    if status != 201 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to create API token")
            .to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    let api_token = body["token"]
        .as_str()
        .ok_or_else(|| CliError::Other("No token in response".into()))?;

    // Store config and credentials
    save_config(&Config {
        server: Some(server.clone()),
    })?;
    save_credentials(&Credentials {
        token: Some(api_token.to_string()),
    })?;

    println!("Logged in to {} successfully.", server);
    println!("API token stored locally (name: {}).", token_name);

    Ok(())
}

/// Show current authentication status.
pub async fn status(server_override: Option<&str>, token_override: Option<&str>) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let (status, body) = client.request("GET", "/api/v1/auth/me", None).await?;

    if status != 200 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to get user info")
            .to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    println!("Server:     {}", server);
    println!(
        "User:       {} {}",
        body["first_name"].as_str().unwrap_or(""),
        body["last_name"].as_str().unwrap_or("")
    );
    println!("Email:      {}", body["email"].as_str().unwrap_or(""));
    println!("User ID:    {}", body["id"].as_str().unwrap_or(""));
    if let Some(roles) = body["roles"].as_array() {
        let role_names: Vec<&str> = roles.iter().filter_map(|r| r.as_str()).collect();
        println!("Roles:      {}", role_names.join(", "));
    }

    Ok(())
}

/// Create a new API token.
pub async fn token_create(
    name: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let create_body = serde_json::json!({ "name": name });
    let (status, body) = client
        .request("POST", "/api/v1/auth/api-tokens", Some(create_body))
        .await?;

    if status != 201 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to create token")
            .to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    println!("Token created:");
    println!("  ID:    {}", body["id"].as_str().unwrap_or(""));
    println!("  Token: {}", body["token"].as_str().unwrap_or(""));
    println!();
    println!("Store this token securely - it will not be shown again.");

    Ok(())
}

/// List API tokens.
pub async fn token_list(server_override: Option<&str>, token_override: Option<&str>) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let (status, body) = client
        .request("GET", "/api/v1/auth/api-tokens", None)
        .await?;

    if status != 200 {
        let msg = body.as_str().unwrap_or("Failed to list tokens").to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    let tokens = body["tokens"].as_array();
    match tokens {
        Some(tokens) if !tokens.is_empty() => {
            println!("{:<38} {:<30} {:<25} STATUS", "ID", "NAME", "CREATED");
            for t in tokens {
                let status_str = if t["revoked_at"].is_string() {
                    "revoked"
                } else {
                    "active"
                };
                println!(
                    "{:<38} {:<30} {:<25} {}",
                    t["id"].as_str().unwrap_or(""),
                    t["name"].as_str().unwrap_or(""),
                    t["created_at"].as_str().unwrap_or(""),
                    status_str,
                );
            }
        }
        _ => {
            println!("No API tokens found.");
        }
    }

    Ok(())
}

/// Revoke an API token.
pub async fn token_revoke(
    id: &str,
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<()> {
    let (server, token) = resolve_auth(server_override, token_override)?;
    let client = ApiClient::with_token(&server, &token)?;

    let path = format!("/api/v1/auth/api-tokens/{id}");
    let (status, body) = client.request("DELETE", &path, None).await?;

    if status != 204 {
        let msg = body
            .as_str()
            .unwrap_or("Failed to revoke token")
            .to_string();
        return Err(CliError::Api {
            status,
            message: msg,
        });
    }

    println!("Token {} revoked.", id);

    Ok(())
}

/// Resolve server and token from overrides or stored config.
fn resolve_auth(
    server_override: Option<&str>,
    token_override: Option<&str>,
) -> Result<(String, String)> {
    let config = load_config()?;
    let creds = load_credentials()?;

    let server = server_override
        .map(|s| s.to_string())
        .or(config.server)
        .ok_or(CliError::NotLoggedIn)?;

    let token = token_override
        .map(|t| t.to_string())
        .or(creds.token)
        .ok_or(CliError::NotLoggedIn)?;

    Ok((server, token))
}

fn prompt(msg: &str) -> Result<String> {
    eprint!("{}", msg);
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    Ok(input.trim().to_string())
}

fn chrono_date() -> String {
    // Simple date string YYYY-MM-DD without pulling in chrono
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Days since epoch
    let days = now / 86400;
    // Approximate date calculation
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year {
            break;
        }
        remaining -= days_in_year;
        y += 1;
    }
    let month_days: &[i64] = if is_leap(y) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut m = 1;
    for &md in month_days {
        if remaining < md {
            break;
        }
        remaining -= md;
        m += 1;
    }
    let d = remaining + 1;
    format!("{y:04}-{m:02}-{d:02}")
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}
