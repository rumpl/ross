use ross_core::ross::ross_client::RossClient;
use ross_core::ross::HealthCheckRequest;

pub async fn health_check(addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let mut client = RossClient::connect(addr.to_string()).await.map_err(|e| {
        format!(
            "Failed to connect to daemon at {}: {}. Is the daemon running?",
            addr, e
        )
    })?;

    let response = client
        .health_check(HealthCheckRequest {})
        .await
        .map_err(|e| format!("Health check failed: {}", e))?;

    let health = response.into_inner();

    println!("Daemon Status:");
    println!(
        "  Healthy: {}",
        if health.healthy { "✓ yes" } else { "✗ no" }
    );
    println!("  Version: {}", health.version);

    Ok(())
}
