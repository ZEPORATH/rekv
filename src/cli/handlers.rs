use std::path::Path;
use serde_json::Value;

use crate::api::grpc::{GetRequest, RekvServiceClient, SetRequest, WatchRequest};
use crate::api::uds::{send_uds_request, UdsRequest, UdsResponse};
use crate::constants::{DEFAULT_GRPC_ADDRESS, JSON_NULL};

/// Execute CLI GET request (tries UDS first, falls back to gRPC).
pub async fn execute_get(
    path: &str,
    address: Option<&str>,
    uds_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    // 1. If explicit address provided, connect over gRPC
    if let Some(addr) = address {
        return execute_get_grpc(path, addr).await;
    }

    // 2. Try UDS if socket file exists
    if uds_path.exists() {
        match execute_get_uds(path, uds_path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::debug!("UDS get failed ({}), falling back to gRPC...", e);
            }
        }
    }

    // 3. Fallback to default local gRPC port
    execute_get_grpc(path, DEFAULT_GRPC_ADDRESS).await
}

async fn execute_get_uds(path: &str, uds_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let req = UdsRequest::Get {
        path: path.to_string(),
    };
    let resp = send_uds_request(uds_path, &req).await?;

    match resp {
        UdsResponse::Ok {
            found,
            value,
            ..
        } => {
            if found.unwrap_or(false) {
                if let Some(val) = value {
                    print_formatted_json(&val);
                }
            } else {
                eprintln!("Path not found: {}", path);
                std::process::exit(1);
            }
        }
        UdsResponse::Error { error } => {
            eprintln!("Error: {}", error);
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn execute_get_grpc(path: &str, address: &str) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = if address.starts_with("http://") || address.starts_with("https://") {
        address.to_string()
    } else {
        format!("http://{}", address)
    };

    let channel = tonic::transport::Channel::from_shared(endpoint)?
        .connect()
        .await?;
    let mut client = RekvServiceClient::new(channel);

    let resp = client
        .get(GetRequest {
            path: path.to_string(),
        })
        .await?
        .into_inner();

    if resp.found {
        if let Ok(val) = serde_json::from_str::<Value>(&resp.json_value) {
            print_formatted_json(&val);
        } else {
            println!("{}", resp.json_value);
        }
    } else {
        eprintln!("Path not found: {}", path);
        std::process::exit(1);
    }

    Ok(())
}

/// Execute CLI SET request (tries UDS first, falls back to gRPC).
pub async fn execute_set(
    path: &str,
    value_str: &str,
    address: Option<&str>,
    uds_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let json_val = parse_or_string_value(value_str);

    // 1. If explicit address provided, connect over gRPC
    if let Some(addr) = address {
        return execute_set_grpc(path, &json_val, addr).await;
    }

    // 2. Try UDS if socket file exists
    if uds_path.exists() {
        match execute_set_uds(path, json_val.clone(), uds_path).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                log::debug!("UDS set failed ({}), falling back to gRPC...", e);
            }
        }
    }

    // 3. Fallback to default local gRPC port
    execute_set_grpc(path, &json_val, DEFAULT_GRPC_ADDRESS).await
}

async fn execute_set_uds(
    path: &str,
    value: Value,
    uds_path: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let req = UdsRequest::Set {
        path: path.to_string(),
        value,
    };
    let resp = send_uds_request(uds_path, &req).await?;

    match resp {
        UdsResponse::Ok {
            affected_count,
            affected_paths,
            ..
        } => {
            println!("OK ({} path(s) updated)", affected_count.unwrap_or(0));
            if let Some(paths) = affected_paths {
                for p in paths {
                    println!("  -> {}", p);
                }
            }
        }
        UdsResponse::Error { error } => {
            eprintln!("Error: {}", error);
            std::process::exit(1);
        }
    }
    Ok(())
}

async fn execute_set_grpc(
    path: &str,
    value: &Value,
    address: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let endpoint = if address.starts_with("http://") || address.starts_with("https://") {
        address.to_string()
    } else {
        format!("http://{}", address)
    };

    let channel = tonic::transport::Channel::from_shared(endpoint)?
        .connect()
        .await?;
    let mut client = RekvServiceClient::new(channel);

    let json_value_str = serde_json::to_string(value)?;
    let resp = client
        .set(SetRequest {
            path: path.to_string(),
            json_value: json_value_str,
        })
        .await?
        .into_inner();

    if resp.success {
        println!("OK ({} path(s) updated)", resp.affected_count);
        for p in resp.affected_paths {
            println!("  -> {}", p);
        }
    } else {
        eprintln!("Error: {}", resp.error);
        std::process::exit(1);
    }

    Ok(())
}

fn parse_or_string_value(s: &str) -> Value {
    if let Ok(v) = serde_json::from_str::<Value>(s) {
        v
    } else {
        Value::String(s.to_string())
    }
}

fn print_formatted_json(val: &Value) {
    match val {
        Value::String(s) => println!("{}", s),
        Value::Number(n) => println!("{}", n),
        Value::Bool(b) => println!("{}", b),
        Value::Null => println!("{}", JSON_NULL),
        _ => {
            if let Ok(pretty) = serde_json::to_string_pretty(val) {
                println!("{}", pretty);
            } else {
                println!("{}", val);
            }
        }
    }
}

/// Execute CLI WATCH request streaming live configuration change events over gRPC.
pub async fn execute_watch(
    path: &str,
    address: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let target_addr = address.unwrap_or(DEFAULT_GRPC_ADDRESS);
    let endpoint = if target_addr.starts_with("http://") || target_addr.starts_with("https://") {
        target_addr.to_string()
    } else {
        format!("http://{}", target_addr)
    };

    println!("Watching for changes on '{}' via {}...", path, endpoint);

    let channel = tonic::transport::Channel::from_shared(endpoint)?
        .connect()
        .await?;
    let mut client = RekvServiceClient::new(channel);

    let mut stream = client
        .watch(WatchRequest {
            path: path.to_string(),
        })
        .await?
        .into_inner();

    println!("Streaming live events (press Ctrl+C to exit):");

    while let Some(event) = stream.message().await? {
        let old_repr = if event.old_value.is_empty() {
            "<none>".to_string()
        } else {
            event.old_value
        };
        println!(
            "[{}] {} -> {} (was: {})",
            event.timestamp_ms,
            event.path,
            event.new_value,
            old_repr
        );
    }

    Ok(())
}
