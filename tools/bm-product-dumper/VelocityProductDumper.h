#pragma once

#include "bakkesmod/plugin/bakkesmodplugin.h"
#include <filesystem>
#include <string>
#include <vector>
#include <map>
#include <set>

constexpr auto plugin_version = "1.3";

static const std::vector<std::string> PAINT_NAMES = {
    "None",
    "Crimson",
    "Lime",
    "Black",
    "Orange",
    "Sky Blue",
    "Cobalt",
    "Saffron",
    "Grey",
    "Pink",
    "Forest Green",
    "Purple",
    "Titanium White",
    "Burnt Sienna",
    "Gold",
    "Rose Gold",
    "White Gold",
    "Onyx",
    "Platinum"
};

static const std::map<int, std::string> SLOT_NAMES = {
    {0, "Body"},
    {1, "Decal"},
    {2, "Wheels"},
    {3, "Rocket Boost"},
    {4, "Antenna"},
    {5, "Topper"},
    {6, "Paint Finish"},
    {7, "Trail"},
    {8, "Goal Explosion"},
    {9, "Banner"},
    {10, "Avatar Border"},
    {11, "Engine Audio"},
    {12, "Title"},
};

static const std::map<int, std::string> QUALITY_NAMES = {
    {0, "Common"},
    {1, "Uncommon"},
    {2, "Rare"},
    {3, "VeryRare"},
    {4, "Import"},
    {5, "Exotic"},
    {6, "BlackMarket"},
    {7, "Premium"},
    {8, "Limited"},
    {9, "Legacy"},
};

struct ProductEntry {
    int product_id = 0;
    std::string label = "";
    std::string long_label = "";
    std::string slot = "";
    int slot_id = -1;
    std::string quality = "";
    int quality_id = -1;
    bool paintable = false;
    std::string asset_package = "";
    std::string asset_path = "";
    std::string unlock_method = "";

    std::vector<int> paint_ids;
};

class VelocityProductDumper : public BakkesMod::Plugin::BakkesModPlugin
{
public:
    void onLoad() override;
    void onUnload() override;

private:
    void DumpAllProducts(std::vector<std::string> args);
    void DumpPaintableOnly(std::vector<std::string> args);
    void DoDump(bool paintableOnly);

    std::filesystem::path GetOutputDir();
    std::string SlotName(int id);
    std::string QualityName(int id);
    std::string PaintName(int id);
    std::string ExtractYear(const std::string& s);
    std::string EscapeJson(const std::string& s);
};
