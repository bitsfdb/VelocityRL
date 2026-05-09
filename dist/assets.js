import * as fs from 'fs';
import * as path from 'path';
import stringSimilarity from 'string-similarity';
import { fileURLToPath } from 'url';
const __dirname = path.dirname(fileURLToPath(import.meta.url));
// Load and transform items.json into ASSET_MAP
const ITEMS_JSON_PATH = path.join(__dirname, '../python/items.json');
let loadedAssets = [];
try {
    if (fs.existsSync(ITEMS_JSON_PATH)) {
        const rawData = JSON.parse(fs.readFileSync(ITEMS_JSON_PATH, 'utf8'));
        if (rawData.Items) {
            loadedAssets = rawData.Items.map((p) => ({
                name: p["Product"],
                productId: p["ID"],
                packageName: p["AssetPackage"] ? p["AssetPackage"].replace('.upk', '') : '',
                assetPath: p["AssetPath"],
                slot: p["Slot"]
            })).filter((a) => a.packageName !== '');
        }
    }
}
catch (e) {
    console.warn(" Warning: Failed to load items.json. Using minimal fallback list.");
}
export const ASSET_MAP = loadedAssets.length > 0 ? loadedAssets : [
    { name: "Standard Boost", packageName: "Standard_Boost_SF", productId: 63 },
    { name: "Flamethrower", packageName: "Flamethrower_SF", productId: 36 },
    { name: "(Alpha Reward) Gold Rush", packageName: "Boost_AlphaReward_SF", productId: 32 },
    { name: "Octane", packageName: "Body_Octane_SF", productId: 23 },
];
/**
 * Performs a fuzzy/case-insensitive search for an item name or product ID.
 * If mapping fails, it scans the provided directory for potential matches.
 */
export async function resolvePackagePath(query, cookedDir) {
    const q = query.toLowerCase();
    // 1. Check Internal Map first
    let match = ASSET_MAP.find(a => a.name.toLowerCase() === q || a.productId?.toString() === q);
    if (match) {
        const fullPath = path.join(cookedDir, `${match.packageName}.upk`);
        if (fs.existsSync(fullPath)) {
            return { path: fullPath, name: match.packageName };
        }
    }
    // 2. Directory Scan + Keyword Search
    if (!fs.existsSync(cookedDir)) {
        console.warn(` Warning: CookedPCConsole directory not found at: ${cookedDir}`);
        return null;
    }
    const files = fs.readdirSync(cookedDir)
        .filter(f => f.endsWith('.upk'))
        .filter(f => !f.toLowerCase().includes('t_sf') && !f.toLowerCase().includes('sf_t'));
    const keywords = q.split(' ').filter(k => k.length > 2); // only words > 2 chars
    // Find files containing ALL keywords
    let keywordMatches = files.filter(f => {
        const lowerF = f.toLowerCase();
        return keywords.every(k => lowerF.includes(k));
    });
    if (keywordMatches.length === 1) {
        return {
            path: path.join(cookedDir, keywordMatches[0]),
            name: keywordMatches[0].replace('.upk', '')
        };
    }
    else if (keywordMatches.length > 1) {
        return { candidates: keywordMatches };
    }
    // 3. String Similarity (Fuzzy Matching)
    const matches = stringSimilarity.findBestMatch(q, files);
    const topMatches = matches.ratings
        .sort((a, b) => b.rating - a.rating)
        .slice(0, 5)
        .filter(m => m.rating > 0.3); // Threshold for sanity
    if (topMatches.length > 0) {
        // If the best match is very high, return it, otherwise return candidates
        if (topMatches[0].rating > 0.8) {
            return {
                path: path.join(cookedDir, topMatches[0].target),
                name: topMatches[0].target.replace('.upk', '')
            };
        }
        return { candidates: topMatches.map(m => m.target) };
    }
    return null;
}
export function getClosestFiles(query, cookedDir) {
    if (!fs.existsSync(cookedDir))
        return [];
    const files = fs.readdirSync(cookedDir)
        .filter(f => f.endsWith('.upk'))
        .filter(f => !f.toLowerCase().includes('t_sf') && !f.toLowerCase().includes('sf_t'));
    const matches = stringSimilarity.findBestMatch(query.toLowerCase(), files);
    return matches.ratings
        .sort((a, b) => b.rating - a.rating)
        .slice(0, 5)
        .map(m => m.target);
}
export function searchAssets(term) {
    const q = term.toLowerCase();
    return ASSET_MAP.filter(a => {
        const nameMatch = a.name.toLowerCase().includes(q);
        const pkgMatch = a.packageName && a.packageName.toLowerCase().includes(q);
        const idMatch = a.productId?.toString() === q;
        const slotMatch = a.slot?.toLowerCase().includes(q);
        const matches = nameMatch || pkgMatch || idMatch || slotMatch;
        const isThumbnail = a.packageName && (a.packageName.toLowerCase().includes('t_sf') || a.packageName.toLowerCase().includes('sf_t'));
        return matches && !isThumbnail;
    }).slice(0, 20); // Limit to top 20 for CLI readability
}
