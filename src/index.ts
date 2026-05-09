#!/usr/bin/env node
import { Command } from 'commander';
import axios from 'axios';
import * as fs from 'fs';
import * as path from 'path';
import { execSync } from 'child_process';
import inquirer from 'inquirer';
import { UPKFile } from './upk.js';
import { resolvePackagePath, searchAssets, getClosestFiles, ASSET_MAP } from './assets.js';
import { UPKSwapper } from './swapper.js';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const program = new Command();

program
  .name('RLItemMod')
  .description('Rocket League Surgical UPK Patcher')
  .version('1.3.0');

const API_ENDPOINT = 'https://dank/rlapi';
const DEFAULT_COOKED_DIR = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole';

/**
 * Interactive Wizard Flow
 */
async function runInteractiveWizard() {
    let active = true;
    while (active) {
        console.clear();
        console.log('Welcome to RLItemMod, What would you like to do?');
        console.log('-------------------------------------------------');

        const { action } = await inquirer.prompt([{
            type: 'rawlist',
            name: 'action',
            message: 'What would you like to do?',
            choices: [
                { name: 'Swap Item (Visual & Offset Shifting)', value: 'swap' },
                { name: 'Search Asset Database', value: 'search' },
                { name: 'Restore Backups', value: 'restore' },
                { name: 'Configure Game Directory', value: 'config' },
                { name: 'Exit', value: 'exit' }
            ]
        }]);

        if (action === 'exit') {
            active = false;
            break;
        }

        if (action === 'config') {
            const { newDir } = await inquirer.prompt([{
                type: 'input',
                name: 'newDir',
                message: 'Path to CookedPCConsole:',
                default: (global as any).COOKED_DIR || DEFAULT_COOKED_DIR
            }]);
            (global as any).COOKED_DIR = newDir;
            console.log(`✅ Game directory updated to: ${newDir}`);
            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }

        if (action === 'search') {
            const { term } = await inquirer.prompt([{
                type: 'input',
                name: 'term',
                message: 'Enter search term (Item Name or Package):'
            }]);
            const results = searchAssets(term);
            console.table(results);
            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }



        if (action === 'swap') {
            const cookedDir = (global as any).COOKED_DIR || DEFAULT_COOKED_DIR;
            
            console.log('\n--- STEP 1: Target Item (The one you OWN) ---');
            const target = await promptForItemAndUPK('Search for your OWNED item:', cookedDir);
            if (!target) {
                await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
                continue;
            }

            console.log('\n--- STEP 2: Source Item (The one you WANT) ---');
            const source = await promptForItemAndUPK('Search for the item you WANT:', cookedDir);
            if (!source) {
                await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
                continue;
            }

            console.log(`\n🔄 Swapping ${target.item.name} for ${source.item.name}...`);
            const targetUpk = new UPKFile(target.path);
            try {

                const pythonScriptPath = path.resolve(__dirname, '../python/rl_asset_swapper.py');
                backupFile(target.path);
                
                const cmd = `python "${pythonScriptPath}" --no-gui --target "${target.item.name}" --donor "${source.item.name}" --no-preserve-header-offsets --overwrite --donor-dir "${cookedDir}" --output-dir "${cookedDir}"`;
                console.log(`\n⚙️ Executing advanced Python offset-shifter...`);
                execSync(cmd, { stdio: 'inherit' });
                
                console.log('✅ SUCCESS: Visual Swap complete! please restart your game and view your new item!');
            } catch (e: any) {
                console.error(`❌ Failed: ${e.message}`);
            }

            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }

        if (action === 'restore') {
            const cookedDir = (global as any).COOKED_DIR || DEFAULT_COOKED_DIR;
            console.log('\n--- Restoring Backups ---');
            try {
                const files = fs.readdirSync(cookedDir);
                const backups = files.filter(f => f.endsWith('.bak'));
                if (backups.length === 0) {
                    console.log('💡 No backups found.');
                } else {
                    for (const bak of backups) {
                        const original = bak.replace('.bak', '');
                        const bakPath = path.join(cookedDir, bak);
                        const originalPath = path.join(cookedDir, original);
                        
                        console.log(`🔄 Restoring ${original}...`);
                        fs.copyFileSync(bakPath, originalPath);
                        fs.unlinkSync(bakPath);
                    }
                    console.log('✅ SUCCESS: All backups restored!');
                }
            } catch (e: any) {
                console.error(`❌ Failed: ${e.message}`);
            }
            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }


    }
}

/**
 * Helper to handle the "Search -> Refine -> Pick" flow for a UPK item
 */
async function promptForItemAndUPK(message: string, cookedDir: string) {
    const { searchTerm } = await inquirer.prompt([{
        type: 'input',
        name: 'searchTerm',
        message
    }]);

    const matches = searchAssets(searchTerm);
    if (matches.length === 0) {
        console.error(`❌ No items found matching "${searchTerm}".`);
        return null;
    }

    const { selectedItem } = await inquirer.prompt([{
        type: 'rawlist',
        name: 'selectedItem',
        message: 'Select the item:',
        choices: matches.map(m => ({ 
            name: `${m.name} [ID: ${m.productId}] (${m.packageName})`, 
            value: m 
        }))
    }]);

    const owned = selectedItem.productId.toString();
    const result = await resolvePackagePath(owned, cookedDir);

    if (!result) {
        console.error(`❌ Could not resolve "${owned}".`);
        return null;
    }

    let finalUpkPath = '';
    if ('candidates' in result) {
        let currentCandidates = result.candidates;
        while (currentCandidates.length > 20) {
            console.log(`💡 Found ${currentCandidates.length} potential matches.`);
            const { refine } = await inquirer.prompt([{
                type: 'input',
                name: 'refine',
                message: 'Too many files found! Enter a sub-keyword to narrow it down (or press Enter to see all):'
            }]);
            if (!refine) break;
            const filtered = currentCandidates.filter(c => c.toLowerCase().includes(refine.toLowerCase()));
            if (filtered.length > 0) currentCandidates = filtered;
        }

        const { selected } = await inquirer.prompt([{
            type: 'rawlist',
            name: 'selected',
            message: `Multiple matches found. Select one (Showing top ${Math.min(currentCandidates.length, 50)}):`,
            choices: currentCandidates.slice(0, 50)
        }]);
        finalUpkPath = path.join(cookedDir, selected);
    } else {
        finalUpkPath = result.path;
    }

    return { path: finalUpkPath, item: selectedItem };
}

async function executePatch(upkPath: string, data: string | Buffer, exportIndex: number) {
    try {
        console.log(`🚀 Patching ${upkPath}...`);
        backupFile(upkPath);
        const upk = new UPKFile(upkPath);
        
        upk.readSummary();
        upk.readExportMap();

        if (exportIndex === -1) {
            exportIndex = upk.exports.reduce((maxIdx, curr, idx, arr) => 
                curr.serialSize > arr[maxIdx].serialSize ? idx : maxIdx, 0);
            console.log(`💡 Auto-selected Export[${exportIndex}] (Size: ${upk.exports[exportIndex].serialSize} bytes)`);
        }

        const newHex = (typeof data === 'string') ? fs.readFileSync(data) : data;
        upk.patchExport(exportIndex, newHex);
        console.log('✅ SUCCESS: Patch complete!');
    } catch (error: any) {
        console.error('❌ CRITICAL FAILURE:', error.message);
    }
}

function backupFile(filePath: string) {
    const backupPath = `${filePath}.bak`;
    if (!fs.existsSync(backupPath)) {
        console.log(`💾 Creating backup: ${path.basename(backupPath)}`);
        fs.copyFileSync(filePath, backupPath);
    }
}

// --- CLI COMMANDS ---

program
  .command('list')
  .description('Shows user\'s owned items from the API')
  .action(async () => {
    // Existing list logic...
    console.log('Fetching items... (Placeholder)');
  });

program
  .command('search')
  .argument('<term>', 'Keyword to search for')
  .action((term) => {
    const results = searchAssets(term);
    console.table(results);
  });

program
  .command('swap')
  .requiredOption('--owned <itemName>', 'Item name or Product ID')
  .requiredOption('--target <targetHexFile>', 'Hex file path')
  .option('--upk <path>', 'Explicit UPK path')
  .option('--dir <path>', 'Cooked dir', DEFAULT_COOKED_DIR)
  .option('--export <index>', 'Export index')
  .action(async (options) => {
      // Call executePatch with resolved paths...
      let upkPath = options.upk;
      let cookedDir = options.dir;

      if (!fs.existsSync(cookedDir)) {
          console.error(`❌ Error: CookedPCConsole directory not found at: ${cookedDir}`);
          console.log('Use --dir <path> to specify the correct game directory.');
          return;
      }

      if (!upkPath) {
          const res = await resolvePackagePath(options.owned, cookedDir);
          if (res && 'path' in res) upkPath = res.path;
      }

      if (!upkPath) {
          console.error(`❌ Error: Could not resolve item "${options.owned}" in ${cookedDir}.`);
          return;
      }
      await executePatch(upkPath, options.target, options.export ? parseInt(options.export) : -1);
  });

// Handle Ctrl+C gracefully
process.on('SIGINT', () => {
    console.log('\n👋 Exiting RLItemMod. See you next time!');
    process.exit(0);
});

async function runSafeWizard() {
    try {
        await runInteractiveWizard();
    } catch (e: any) {
        if (e.name === 'ExitPromptError') {
            console.log('\n👋 Goodbye!');
        } else {
            console.error('\n💥 An unexpected error occurred:', e.message);
        }
        process.exit(0);
    }
}

// Default to wizard if no command is provided
if (process.argv.length <= 2) {
    runSafeWizard();
} else {
    program.parse(process.argv);
}

