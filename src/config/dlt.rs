use serde::{Deserialize, Serialize};

pub const DEFAULT_DLT_PORT: u16 = 3490;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, schemars::JsonSchema)]
pub struct DltDevice {
    pub name: String,
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
}

impl DltDevice {
    pub fn save(device: &DltDevice) -> Result<(), String> {
        let config_path = dirs::config_dir()
            .map(|d| d.join("logana").join("config.json"))
            .ok_or_else(|| "Could not determine config directory".to_string())?;
        if !config_path.exists()
            && let Some(parent) = config_path.parent()
        {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create config dir: {e}"))?;
        }
        Self::save_to(device, &config_path)
    }

    pub fn remove(device_name: &str) -> Result<(), String> {
        let config_path = dirs::config_dir()
            .map(|d| d.join("logana").join("config.json"))
            .ok_or_else(|| "Could not determine config directory".to_string())?;
        Self::remove_from(device_name, &config_path)
    }

    pub(crate) fn save_to(device: &DltDevice, config_path: &std::path::Path) -> Result<(), String> {
        let mut value: serde_json::Value = if config_path.exists() {
            let contents =
                std::fs::read_to_string(config_path).map_err(|e| format!("Read error: {e}"))?;
            serde_json::from_str(&contents).unwrap_or_else(|_| serde_json::json!({}))
        } else {
            serde_json::json!({})
        };

        let device_val =
            serde_json::to_value(device).map_err(|e| format!("Serialize error: {e}"))?;

        match value.get_mut("dlt_devices") {
            Some(arr) if arr.is_array() => {
                arr.as_array_mut().unwrap().push(device_val);
            }
            _ => {
                value["dlt_devices"] = serde_json::json!([device_val]);
            }
        }

        let json =
            serde_json::to_string_pretty(&value).map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(config_path, json).map_err(|e| format!("Write error: {e}"))?;
        Ok(())
    }

    pub(crate) fn remove_from(
        device_name: &str,
        config_path: &std::path::Path,
    ) -> Result<(), String> {
        if !config_path.exists() {
            return Ok(());
        }

        let contents =
            std::fs::read_to_string(config_path).map_err(|e| format!("Read error: {e}"))?;
        let mut value: serde_json::Value =
            serde_json::from_str(&contents).unwrap_or_else(|_| serde_json::json!({}));

        if let Some(arr) = value.get_mut("dlt_devices").and_then(|v| v.as_array_mut()) {
            arr.retain(|d| d.get("name").and_then(|n| n.as_str()) != Some(device_name));
        }

        let json =
            serde_json::to_string_pretty(&value).map_err(|e| format!("Serialize error: {e}"))?;
        std::fs::write(config_path, json).map_err(|e| format!("Write error: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, host: &str, port: Option<u16>) -> DltDevice {
        DltDevice {
            name: name.to_string(),
            host: host.to_string(),
            port,
        }
    }

    fn write_config(path: &std::path::Path, content: &str) {
        std::fs::write(path, content).unwrap();
    }

    fn read_config(path: &std::path::Path) -> serde_json::Value {
        let s = std::fs::read_to_string(path).unwrap();
        serde_json::from_str(&s).unwrap()
    }

    #[test]
    fn test_deserialize_explicit_port() {
        let json = r#"{"name":"dev","host":"192.168.1.1","port":5000}"#;
        let d: DltDevice = serde_json::from_str(json).unwrap();
        assert_eq!(d.name, "dev");
        assert_eq!(d.host, "192.168.1.1");
        assert_eq!(d.port, Some(5000));
    }

    #[test]
    fn test_deserialize_absent_port_is_none() {
        let json = r#"{"name":"dev","host":"localhost"}"#;
        let d: DltDevice = serde_json::from_str(json).unwrap();
        assert_eq!(d.port, None);
        assert_eq!(d.port.unwrap_or(DEFAULT_DLT_PORT), DEFAULT_DLT_PORT);
    }

    #[test]
    fn test_serialize_round_trip() {
        let original = device("car-ecu", "10.0.0.1", Some(3490));
        let json = serde_json::to_string(&original).unwrap();
        let restored: DltDevice = serde_json::from_str(&json).unwrap();
        assert_eq!(original, restored);
    }

    #[test]
    fn test_clone_and_equality() {
        let a = device("a", "h", Some(3490));
        let b = a.clone();
        assert_eq!(a, b);
    }

    #[test]
    fn test_save_to_creates_new_file_with_device() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        DltDevice::save_to(&device("ecu", "10.0.0.1", Some(3490)), &path).unwrap();

        let v = read_config(&path);
        assert_eq!(v["dlt_devices"].as_array().unwrap().len(), 1);
        assert_eq!(v["dlt_devices"][0]["name"], "ecu");
        assert_eq!(v["dlt_devices"][0]["host"], "10.0.0.1");
        assert_eq!(v["dlt_devices"][0]["port"], 3490);
    }

    #[test]
    fn test_save_to_appends_to_existing_array() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(
            &path,
            r#"{"dlt_devices":[{"name":"a","host":"h1","port":3490}]}"#,
        );

        DltDevice::save_to(&device("b", "h2", Some(3491)), &path).unwrap();

        let v = read_config(&path);
        let arr = v["dlt_devices"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[1]["name"], "b");
    }

    #[test]
    fn test_save_to_preserves_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(&path, r#"{"theme":"dark"}"#);

        DltDevice::save_to(&device("ecu", "localhost", None), &path).unwrap();

        let v = read_config(&path);
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["dlt_devices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_save_to_replaces_non_array_dlt_devices() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(&path, r#"{"dlt_devices":"bad"}"#);

        DltDevice::save_to(&device("ecu", "localhost", None), &path).unwrap();

        let v = read_config(&path);
        assert!(v["dlt_devices"].is_array());
        assert_eq!(v["dlt_devices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_save_to_handles_invalid_json_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(&path, "not-json");

        DltDevice::save_to(&device("ecu", "localhost", None), &path).unwrap();

        let v = read_config(&path);
        assert_eq!(v["dlt_devices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_remove_from_missing_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        assert!(DltDevice::remove_from("ecu", &path).is_ok());
        assert!(!path.exists());
    }

    #[test]
    fn test_remove_from_removes_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(
            &path,
            r#"{"dlt_devices":[{"name":"alpha","host":"h1","port":3490},{"name":"beta","host":"h2","port":3491}]}"#,
        );

        DltDevice::remove_from("alpha", &path).unwrap();

        let v = read_config(&path);
        let arr = v["dlt_devices"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["name"], "beta");
    }

    #[test]
    fn test_remove_from_missing_name_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(
            &path,
            r#"{"dlt_devices":[{"name":"alpha","host":"h","port":3490}]}"#,
        );

        DltDevice::remove_from("missing", &path).unwrap();

        let v = read_config(&path);
        assert_eq!(v["dlt_devices"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_remove_from_no_dlt_devices_key_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        write_config(&path, r#"{"theme":"dark"}"#);

        DltDevice::remove_from("ecu", &path).unwrap();

        let v = read_config(&path);
        assert_eq!(v["theme"], "dark");
        assert!(v.get("dlt_devices").is_none());
    }
}
