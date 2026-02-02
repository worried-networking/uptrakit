use clap::Parser;

/// Uptrakit MQTT Service — standalone MQTT client with lease-based tenant distribution.
#[derive(Parser, Debug)]
#[command(name = "uptrakit-mqtt")]
pub struct Args {
    /// Database URL.
    /// Supported schemes depend on enabled features:
    ///   SQLite (default): sqlite://path/to/db.sqlite
    ///   PostgreSQL: postgresql://user:pass@host:5432/dbname
    ///   MySQL: mysql://user:pass@host:3306/dbname
    #[arg(long)]
    pub db_url: String,

    /// Maximum number of tenants this instance will manage.
    /// 0 means unlimited.
    #[arg(long, default_value = "0")]
    pub max_tenants: u32,

    /// Heartbeat interval in seconds.
    #[arg(long, default_value = "15")]
    pub heartbeat_interval: u64,

    /// Polling interval for new/changed tenants in seconds.
    #[arg(long, default_value = "10")]
    pub poll_interval: u64,

    /// Stale lease timeout in seconds.
    /// Leases with heartbeat_at older than this are considered abandoned.
    #[arg(long, default_value = "60")]
    pub lease_timeout: u64,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::Args;

    #[test]
    fn defaults_parse() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--db-url", "sqlite::memory:"]).unwrap();
        assert_eq!(args.max_tenants, 0);
        assert_eq!(args.heartbeat_interval, 15);
        assert_eq!(args.poll_interval, 10);
        assert_eq!(args.lease_timeout, 60);
    }

    #[test]
    fn custom_values_parsed() {
        let args = Args::try_parse_from([
            "uptrakit-mqtt",
            "--db-url",
            "postgresql://user:pass@host:5432/db",
            "--max-tenants",
            "5",
            "--heartbeat-interval",
            "30",
            "--poll-interval",
            "20",
            "--lease-timeout",
            "120",
        ])
        .unwrap();
        assert_eq!(args.db_url, "postgresql://user:pass@host:5432/db");
        assert_eq!(args.max_tenants, 5);
        assert_eq!(args.heartbeat_interval, 30);
        assert_eq!(args.poll_interval, 20);
        assert_eq!(args.lease_timeout, 120);
    }

    #[test]
    fn max_tenants_zero_default() {
        let args =
            Args::try_parse_from(["uptrakit-mqtt", "--db-url", "sqlite::memory:"]).unwrap();
        assert_eq!(args.max_tenants, 0);
    }

    #[test]
    fn db_url_required() {
        let result = Args::try_parse_from(["uptrakit-mqtt"]);
        assert!(result.is_err());
    }
}
