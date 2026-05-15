use std::fs::File;
use std::io::{BufRead, BufReader};
use serde_json::{json, Value};

pub fn convert_csv_to_json(csv_path: &str) -> Result<Value, String> {
    let file = File::open(csv_path).map_err(|e| e.to_string())?;
    let reader = BufReader::new(file);
    let mut items = Vec::new();

    for line in reader.lines() {
        let line = line.map_err(|e| e.to_string())?;
        let parts: Vec<&str> = line.split(',').collect();
        if parts.len() < 4 { continue; }

        let id = parts[0].parse::<i32>().unwrap_or(0);
        let slot = parts[1];
        let asset_path = parts[2];
        let label = parts[3..].join(","); // Handle commas in labels

        // Simple mapping: Product_TA ProductsDB.Products.Body_Octane -> Body_Octane
        let asset_name = asset_path.split('.').last().unwrap_or(asset_path);
        let package_name = if asset_name.starts_with("Body_") {
            format!("{}_SF", asset_name)
        } else if asset_name.starts_with("Boost_") {
            format!("{}_SF", asset_name)
        } else {
            asset_name.to_string()
        };

        items.push(json!({
            "ID": id,
            "Slot": slot,
            "AssetPackage": package_name,
            "AssetPath": format!("{}.{}", package_name, asset_name),
            "Product": label
        }));
    }

    Ok(json!({ "Items": items }))
}
