#!/usr/bin/env python3
import argparse
import hashlib
import hmac
import json
import os
import sys
import time
from pathlib import Path
from typing import Dict, List, Optional
import requests

# Psynet Signing Keys from dank/rlapi
REQUEST_KEY = bytes.fromhex("c338bd36fb8c42b1a431d30add939fc7")
RESPONSE_KEY = bytes.fromhex("3b932153785842ac927744b292e40e52")
PSYNET_RPC_URL = "https://api.rlpp.psynet.gg/rpc/"

def get_psysig(body: str, key: bytes) -> str:
    # HMAC-SHA256(key, "-" + <request body>)
    msg = f"-{body}".encode("utf-8")
    sig = hmac.new(key, msg, hashlib.sha256).digest()
    import base64
    return base64.b64encode(sig).decode("utf-8")

class PsynetClient:
    def __init__(self, auth_ticket: str):
        self.auth_ticket = auth_ticket
        self.session_id = None
        self.psy_token = None
        self.player_id = None

    def call_rpc(self, service: str, body: dict) -> dict:
        headers = {
            "PsyService": service,
            "PsyEnvironment": "Prod",
            "User-Agent": "RL Win/250811.43331.492665 gzip",
            "Content-Type": "application/json"
        }
        if self.psy_token: headers["PsyToken"] = self.psy_token
        if self.session_id: headers["PsySessionID"] = self.session_id
        
        json_body = json.dumps(body)
        headers["PsySig"] = get_psysig(json_body, REQUEST_KEY)
        
        resp = requests.post(PSYNET_RPC_URL, headers=headers, data=json_body)
        if resp.status_code != 200:
            raise Exception(f"RPC failed: {resp.status_code} - {resp.text}")
        return resp.json()["Result"]

    def login(self, epic_account_id: str):
        print(f"Logging in for {epic_account_id}...")
        body = {
            "Platform": "Epic",
            "PlayerName": epic_account_id,
            "PlayerID": epic_account_id,
            "Language": "INT",
            "AuthTicket": self.auth_ticket,
            "FeatureSet": "PrimeUpdate55_1",
            "Device": "PC",
            "EpicAuthTicket": self.auth_ticket,
            "EpicAccountID": epic_account_id
        }
        res = self.call_rpc("Auth/Login v4", body)
        self.psy_token = res["PsyToken"]
        self.session_id = res["SessionID"]
        print("Login successful.")

    def get_catalog(self, category: str):
        print(f"Fetching catalog for {category}...")
        body = {
            "PlayerID": f"Epic|{self.player_id}|0",
            "Category": category
        }
        return self.call_rpc("Microtransaction/GetCatalog v1", body)

    def get_all_products(self) -> List[dict]:
        # Iterate through common categories to find all products
        categories = ["StarterPack", "Shop", "Blueprint", "TradeIn"]
        all_products = []
        seen_ids = set()
        
        for cat in categories:
            try:
                res = self.get_catalog(cat)
                products = res.get("Products", [])
                for p in products:
                    pid = p.get("ProductID")
                    if pid and pid not in seen_ids:
                        all_products.append(p)
                        seen_ids.add(pid)
            except Exception as e:
                print(f"Warning: Failed to fetch category {cat}: {e}")
        
        return all_products

def map_product_to_item(product: dict) -> dict:
    # Map PsyNet product schema to RLItemMod schema
    return {
        "ID": product.get("ProductID"),
        "Product": product.get("Label", "Unknown"),
        "Quality": product.get("Quality", "Common"),
        "Slot": product.get("Slot", "Unknown"),
        "AssetPackage": "", # Unknown via API
        "AssetPath": "",    # Unknown via API
        "image_url": product.get("Thumbnail", "")
    }

def merge_items(existing_items: List[dict], new_items: List[dict]) -> List[dict]:
    items_map = {str(item.get("ID")): item for item in existing_items}
    
    for new_item in new_items:
        id_str = str(new_item.get("ID"))
        if id_str in items_map:
            # Update metadata but preserve asset paths
            existing = items_map[id_str]
            existing["Product"] = new_item["Product"]
            existing["Quality"] = new_item["Quality"]
            existing["Slot"] = new_item["Slot"]
            if not existing.get("image_url"):
                existing["image_url"] = new_item["image_url"]
        else:
            items_map[id_str] = new_item
            
    return list(items_map.values())

def main():
    parser = argparse.ArgumentParser(description="Rocket League Catalog Fetcher & Database Updater")
    parser.add_argument("--token", help="Epic Exchange Token / Auth Ticket")
    parser.add_argument("--account", help="Epic Account ID")
    parser.add_argument("--output", help="Output JSON file path", default="items.json")
    parser.add_argument("--merge", help="Existing JSON file to merge with")
    args = parser.parse_args()

    token = args.token or os.environ.get("EPIC_TOKEN")
    account = args.account or os.environ.get("EPIC_ACCOUNT", "Unknown")

    if not token:
        print("Error: --token or EPIC_TOKEN env var is required.")
        return

    client = PsynetClient(token)
    try:
        client.login(account)
        products = client.get_all_products()
        print(f"Found {len(products)} unique products.")
        
        new_items = [map_product_to_item(p) for p in products]
        
        final_items = new_items
        if args.merge and os.path.exists(args.merge):
            print(f"Merging with {args.merge}...")
            with open(args.merge, "r", encoding="utf-8-sig") as f:
                raw = json.load(f)
                existing = raw.get("Items", raw if isinstance(raw, list) else [])
                final_items = merge_items(existing, new_items)
        
        output_data = {"Items": final_items}
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(output_data, f, indent=2)
            
        print(f"Successfully updated {args.output} with {len(final_items)} items.")

    except Exception as e:
        print(f"Error: {e}")

if __name__ == "__main__":
    main()
