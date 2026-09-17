#include "pch.h"
#include "VelocityProductDumper.h"
#include "bakkesmod/wrappers/includes.h"
#include "bakkesmod/wrappers/items/attributes/ProductAttribute_PaintedWrapper.h"
#include <fstream>
#include <sstream>
#include <ShlObj.h>

BAKKESMOD_PLUGIN(VelocityProductDumper, "VelocityRL Product Dumper", plugin_version, PLUGINTYPE_FREEPLAY)

void VelocityProductDumper::onLoad()
{
    using namespace std::placeholders;
    cvarManager->registerNotifier(
        "velocity_dump_all",
        std::bind(&VelocityProductDumper::DumpAllProducts, this, _1),
        "Dump ALL products to Downloads/velocity_products_all.json",
        PERMISSION_ALL
    );
    cvarManager->registerNotifier(
        "velocity_dump_paintable",
        std::bind(&VelocityProductDumper::DumpPaintableOnly, this, _1),
        "Dump only PAINTABLE products to Downloads/velocity_products_paintable.json",
        PERMISSION_ALL
    );

    cvarManager->log("=== VelocityRL Product Dumper loaded ===");
    cvarManager->log("  velocity_dump_all       - Dump all items");
    cvarManager->log("  velocity_dump_paintable - Dump only paintable items with paint colors");
}

void VelocityProductDumper::onUnload()
{
    cvarManager->log("VelocityRL Product Dumper unloaded.");
}

std::filesystem::path VelocityProductDumper::GetOutputDir()
{
    char path[MAX_PATH];
    if (SUCCEEDED(SHGetFolderPathA(NULL, CSIDL_PROFILE, NULL, 0, path)))
        return std::filesystem::path(path) / "Downloads";
    return gameWrapper->GetDataFolder();
}

std::string VelocityProductDumper::SlotName(int id)
{
    auto it = SLOT_NAMES.find(id);
    return (it != SLOT_NAMES.end()) ? it->second : ("Unknown_" + std::to_string(id));
}

std::string VelocityProductDumper::QualityName(int id)
{
    auto it = QUALITY_NAMES.find(id);
    return (it != QUALITY_NAMES.end()) ? it->second : ("Unknown_" + std::to_string(id));
}

std::string VelocityProductDumper::PaintName(int id)
{
    if (id >= 0 && id < static_cast<int>(PAINT_NAMES.size()))
        return PAINT_NAMES[id];
    return "Unknown_" + std::to_string(id);
}

std::string VelocityProductDumper::ExtractYear(const std::string& s)
{
    for (size_t i = 0; i + 4 <= s.size(); i++) {
        if (s[i] >= '0' && s[i] <= '9' &&
            s[i + 1] >= '0' && s[i + 1] <= '9' &&
            s[i + 2] >= '0' && s[i + 2] <= '9' &&
            s[i + 3] >= '0' && s[i + 3] <= '9') {
            std::string y = s.substr(i, 4);
            if (y >= "1900" && y <= "2100")
                return y;
        }
    }
    return "";
}

static std::string UnlockMethodName(unsigned char id)
{
    switch (id) {
        case 0: return "UnlockMethod_Default";
        case 1: return "UnlockMethod_Online";
        case 2: return "UnlockMethod_DLC";
        case 3: return "UnlockMethod_Special";
        default: return "UnlockMethod_" + std::to_string(static_cast<int>(id));
    }
}

std::string VelocityProductDumper::EscapeJson(const std::string& s)
{
    std::string out;
    out.reserve(s.size() + 8);
    for (char c : s) {
        switch (c) {
        case '"':  out += "\\\""; break;
        case '\\': out += "\\\\"; break;
        case '\n': out += "\\n";  break;
        case '\r': out += "\\r";  break;
        case '\t': out += "\\t";  break;
        default:   out += c;      break;
        }
    }
    return out;
}

void VelocityProductDumper::DumpAllProducts(std::vector<std::string> args)
{
    DoDump(false);
}

void VelocityProductDumper::DumpPaintableOnly(std::vector<std::string> args)
{
    DoDump(true);
}

void VelocityProductDumper::DoDump(bool paintableOnly)
{
    auto itemsWrapper = gameWrapper->GetItemsWrapper();
    if (itemsWrapper.IsNull()) {
        cvarManager->log("ERROR: ItemsWrapper is null - make sure you're in-game.");
        return;
    }

    auto allProducts = itemsWrapper.GetAllProducts();
    int totalCount = allProducts.Count();
    cvarManager->log("Found " + std::to_string(totalCount) + " total products in game database.");
    if (totalCount == 0) {
        cvarManager->log("ERROR: No products. Make sure the game is fully loaded.");
        return;
    }

    std::map<int, std::set<int>> inventoryPaints;
    try {
        auto ownedProducts = itemsWrapper.GetOwnedProducts();
        int ownedCount = ownedProducts.Count();
        cvarManager->log("Scanning " + std::to_string(ownedCount) + " owned items for painted variants...");

        for (int i = 0; i < ownedCount; i++) {
            try {
                auto onlineItem = ownedProducts.Get(i);
                if (onlineItem.IsNull()) continue;

                int pid = onlineItem.GetProductID();

                auto attrs = onlineItem.GetAttributes();
                int attrCount = attrs.Count();
                for (int a = 0; a < attrCount; a++) {
                    try {
                        auto attr = attrs.Get(a);
                        if (attr.IsNull()) continue;

                        std::string attrType = attr.GetAttributeType();
                        if (attrType.find("Painted") != std::string::npos) {
                            ProductAttribute_PaintedWrapper painted(attr.memory_address);
                            int paintId = painted.GetPaintID();
                            if (paintId > 0 && paintId <= 18) {
                                inventoryPaints[pid].insert(paintId);
                            }
                        }
                    } catch (...) {}
                }
            } catch (...) {}
        }
        cvarManager->log("Found painted variants for " + std::to_string(inventoryPaints.size()) + " unique products from inventory.");
    } catch (...) {
        cvarManager->log("Warning: Could not scan owned items. Paint data will use flags only.");
    }

    std::vector<ProductEntry> entries;
    entries.reserve(totalCount);
    int paintableCount = 0;
    int errorCount = 0;

    for (int i = 0; i < totalCount; i++)
    {
        try {
            auto product = allProducts.Get(i);
            if (product.IsNull()) continue;

            ProductEntry entry;
            entry.product_id = product.GetID();

            try {
                auto lbl = product.GetLabel();
                if (!lbl.IsNull()) entry.label = lbl.ToString();
            } catch (...) {}

            try {
                auto ll = product.GetLongLabel();
                if (!ll.IsNull()) entry.long_label = ll.ToString();
            } catch (...) {}

            try {
                auto slotW = product.GetSlot();
                if (!slotW.IsNull()) {
                    entry.slot_id = slotW.GetSlotIndex();
                    try {
                        auto slotLabel = slotW.GetLabel();
                        if (!slotLabel.IsNull())
                            entry.slot = slotLabel.ToString();
                        else
                            entry.slot = SlotName(entry.slot_id);
                    } catch (...) {
                        entry.slot = SlotName(entry.slot_id);
                    }
                }
            } catch (...) {}

            try {
                entry.quality_id = static_cast<int>(product.GetQuality());
                entry.quality = QualityName(entry.quality_id);
            } catch (...) {}

            try {
                entry.paintable = product.IsPaintable();
                if (entry.paintable) paintableCount++;
            } catch (...) {}

            try {
                unsigned char um = product.GetUnlockMethod();
                entry.unlock_method = UnlockMethodName(um);
            } catch (...) {}

            try {
                entry.asset_package = product.GetAssetPackageName();
            } catch (...) {}

            try {
                auto ap = product.GetAssetPath();
                if (!ap.IsNull()) entry.asset_path = ap.ToString();
            } catch (...) {}

            if (entry.paintable) {
                auto invIt = inventoryPaints.find(entry.product_id);
                if (invIt != inventoryPaints.end() && !invIt->second.empty()) {
                    entry.paint_ids.assign(invIt->second.begin(), invIt->second.end());
                } else {
                    for (int p = 1; p <= 18; p++)
                        entry.paint_ids.push_back(p);
                }
            }

            if (paintableOnly && !entry.paintable) continue;
            entries.push_back(entry);

        } catch (...) {
            errorCount++;
        }
    }

    cvarManager->log("Collected " + std::to_string(entries.size()) + " entries (" +
        std::to_string(paintableCount) + " paintable, " +
        std::to_string(errorCount) + " errors).");

    std::map<std::string, int> labelFreq;
    for (const auto& e : entries) {
        labelFreq[e.label + "\x01" + e.slot]++;
    }

    auto outDir = GetOutputDir();
    std::filesystem::create_directories(outDir);

    std::string filename = paintableOnly
        ? "velocity_products_paintable.json"
        : "velocity_products_all.json";
    auto outPath = outDir / filename;

    std::ofstream ofs(outPath);
    if (!ofs.is_open()) {
        cvarManager->log("ERROR: Could not write to " + outPath.string());
        return;
    }

    ofs << "{\n";
    ofs << "  \"Items\": [\n";

    for (size_t i = 0; i < entries.size(); i++)
    {
        const auto& e = entries[i];
        const bool duplicateLabel = labelFreq[e.label + "\x01" + e.slot] > 1;

        std::string year = ExtractYear(e.asset_package);
        if (year.empty()) year = ExtractYear(e.long_label);

        std::string displayLabel = e.label;
        bool renamed = false;
        if (duplicateLabel) {
            renamed = true;
            if (!year.empty() && e.label.find(year) == std::string::npos) {
                displayLabel = e.label + " (" + year + ")";
            } else {
                displayLabel = e.label + " (ID " + std::to_string(e.product_id) + ")";
            }
        }

        ofs << "    {\n";
        ofs << "      \"ID\": " << e.product_id << ",\n";
        ofs << "      \"Product\": \"" << EscapeJson(displayLabel) << "\",\n";
        if (renamed)
            ofs << "      \"ProductClean\": \"" << EscapeJson(e.label) << "\",\n";
        if (!year.empty())
            ofs << "      \"Year\": \"" << EscapeJson(year) << "\",\n";
        if (!e.long_label.empty() && e.long_label != e.label)
            ofs << "      \"LongLabel\": \"" << EscapeJson(e.long_label) << "\",\n";
        ofs << "      \"Slot\": \"" << EscapeJson(e.slot) << "\",\n";
        ofs << "      \"Quality\": \"" << EscapeJson(e.quality) << "\",\n";
        ofs << "      \"UnlockMethod\": \"" << EscapeJson(e.unlock_method) << "\",\n";
        ofs << "      \"Paintable\": " << (e.paintable ? "true" : "false") << ",\n";

        if (e.paintable && !e.paint_ids.empty()) {
            ofs << "      \"Paints\": [";
            for (size_t p = 0; p < e.paint_ids.size(); p++) {
                int pid = e.paint_ids[p];

                std::string pname = PaintName(pid);

                ofs << "\n        { \"id\": " << pid
                    << ", \"label\": \"" << EscapeJson(pname) << "\" }";
                if (p < e.paint_ids.size() - 1) ofs << ",";
            }
            ofs << "\n      ],\n";
        }

        ofs << "      \"AssetPackage\": \"" << EscapeJson(e.asset_package) << "\",\n";
        ofs << "      \"AssetPath\": \"" << EscapeJson(e.asset_path) << "\"\n";
        ofs << "    }";
        if (i < entries.size() - 1) ofs << ",";
        ofs << "\n";
    }

    ofs << "  ]\n";
    ofs << "}\n";
    ofs.close();

    cvarManager->log("=== DONE! Wrote " + std::to_string(entries.size()) + " items to: " + outPath.string() + " ===");
}
