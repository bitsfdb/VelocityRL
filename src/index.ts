#!/usr/bin/env node
import { Command } from 'commander';
import axios from 'axios';
import * as fs from 'fs';
import * as path from 'path';
import * as os from 'os';
import { spawnSync } from 'child_process';
import inquirer from 'inquirer';
import { UPKFile } from './upk.js';
import { resolvePackagePath, searchAssets, getClosestFiles, ASSET_MAP } from './assets.js';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

const pkg = JSON.parse(fs.readFileSync(path.resolve(__dirname, '../package.json'), 'utf8'));
const APP_VERSION = pkg.version;

const program = new Command();

program
  .name('VelocityRL')
  .description('VelocityRL | Surgical Rocket League Asset Swapping Engine')
  .version(APP_VERSION);

let updateMessage: string | null = null;
const CONFIG_PATH = path.join(os.homedir(), '.velocityrl.json');

function loadConfig() {
    try {
        if (fs.existsSync(CONFIG_PATH)) {
            return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf8'));
        }
    } catch (e) {}
    return {};
}

function saveConfig(config: any) {
    try {
        fs.writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2));
    } catch (e) {}
}

async function ensurePythonDependencies() {
    process.stdout.write('Checking Python engine dependencies... ');
    const check = spawnSync('python', ['-c', 'import cryptography'], { encoding: 'utf8' });
    
    if (check.status !== 0) {
        console.log('\n[!] Missing or broken "cryptography" library. Attempting to repair...');
        const install = spawnSync('python', ['-m', 'pip', 'install', '--upgrade', 'cryptography'], { stdio: 'inherit', shell: true });
        
        if (install.status !== 0) {
            console.log('[!] python -m pip failed, trying python3...');
            spawnSync('python3', ['-m', 'pip', 'install', '--upgrade', 'cryptography'], { stdio: 'inherit', shell: true });
        } else {
            const recheck = spawnSync('python', ['-c', 'import cryptography']);
            if (recheck.status === 0) {
                console.log('SUCCESS: Dependency repaired.');
            } else {
                console.log('WARNING: Library is installed but still failing to import. You may need to restart your terminal.');
            }
        }
    } else {
        console.log('OK');
    }
}

async function checkVersion() {
    try {
        // Checking GitHub Releases for updates
        const res = await axios.get('https://api.github.com/repos/bitsfdb/RLItemMod/releases/latest', { 
            timeout: 3000,
            headers: { 'User-Agent': 'VelocityRL-Updater' }
        });
        const latestTag = res.data.tag_name; // e.g., "v1.1.0"
        const latest = latestTag.replace('v', '');
        
        const l = latest.split('.').map(Number);
        const c = APP_VERSION.split('.').map(Number);
        let isNewer = false;
        for (let i = 0; i < 3; i++) {
            if (l[i] > (c[i] || 0)) { isNewer = true; break; }
            if (l[i] < (c[i] || 0)) break;
        }

        if (isNewer) {
            updateMessage = `\x1b[33m[!] UPDATE AVAILABLE: VelocityRL ${latestTag} is ready!\x1b[0m\n` +
                            `\x1b[33m    Download the new version from: https://github.com/bitsfdb/RLItemMod/releases\x1b[0m\n`;
        }
    } catch (e) {
        // Silently fail if offline or rate-limited
    }
}

const DEFAULT_COOKED_DIR = 'C:\\Program Files\\Epic Games\\RocketLeague\\TAGame\\CookedPCConsole';

async function runInteractiveWizard() {
    let active = true;
    while (active) {
        console.log('\n=================================================');
        if (updateMessage) {
            console.log(updateMessage);
        }
        console.log(`VelocityRL | Version: ${APP_VERSION}`);
        console.log('=================================================');

        const { action } = await inquirer.prompt([{
            type: 'rawlist',
            name: 'action',
            message: 'Main Menu',
            choices: [
                { name: 'Swap Item (Visual & Offset RLing)', value: 'swap' },
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
            saveConfig({ cookedDir: newDir });
            console.log(`Game directory updated to: ${newDir}`);
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
            
            console.log('\n--- STEP 1: Owned Item ---');
            const target = await promptForItemAndUPK('Search for your OWNED item:', cookedDir);
            if (!target) {
                await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
                continue;
            }

            console.log('\n--- STEP 2: Target Visual ---');
            const source = await promptForItemAndUPK('Search for the item you WANT:', cookedDir);
            if (!source) {
                await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
                continue;
            }

            console.log(`\nVelocityRL | Swapping ${target.item.name} -> ${source.item.name}...`);
            try {
                const pythonScriptPath = path.resolve(__dirname, '../python/rl_asset_swapper.py');
                backupFile(target.path);

                const result = spawnSync('python', [
                    pythonScriptPath,
                    '--no-gui',
                    '--target', target.item.productId?.toString() || target.item.packageName,
                    '--donor', source.item.productId?.toString() || source.item.packageName,
                    '--overwrite',
                    '--donor-dir', cookedDir,
                    '--output-dir', cookedDir
                ], { stdio: ['inherit', 'inherit', 'pipe'], encoding: 'utf8' });

                if (result.stderr && result.stderr.trim()) {
                    console.error('\n--- Engine Error Output ---');
                    console.error(result.stderr.trim());
                    console.error('---------------------------');
                }

                if (result.status !== 0) {
                    throw new Error(`Engine process exited with code ${result.status}`);
                }

                console.log('SUCCESS: Visual Swap complete! Restart your game.');
            } catch (e: any) {
                console.error(`Failed: ${e.message}`);
            }

            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }

        if (action === 'restore') {
            const cookedDir = (global as any).COOKED_DIR || DEFAULT_COOKED_DIR;
            console.log(`\n=== VelocityRL RESTORE UTILITY ===`);
            console.log(`Searching in: ${cookedDir}`);
            try {
                if (!fs.existsSync(cookedDir)) {
                    throw new Error(`Directory not found: ${cookedDir}`);
                }
                const files = fs.readdirSync(cookedDir);
                const backups = files.filter(f => f.endsWith('.bak'));
                
                if (backups.length === 0) {
                    console.log('No backups found.');
                } else {
                    const { restoreChoice } = await inquirer.prompt([{
                        type: 'rawlist',
                        name: 'restoreChoice',
                        message: `Found ${backups.length} backups. Select an option:`,
                        choices: [
                            { name: 'Restore ALL items', value: 'all' },
                            ...backups.map(b => ({ name: `Restore ${b.replace('.bak', '')}`, value: b })),
                            { name: 'Cancel', value: 'cancel' }
                        ]
                    }]);

                    if (restoreChoice !== 'cancel') {
                        if (restoreChoice === 'all') {
                            for (const bak of backups) {
                                const original = bak.replace('.bak', '');
                                fs.copyFileSync(path.join(cookedDir, bak), path.join(cookedDir, original));
                                fs.unlinkSync(path.join(cookedDir, bak));
                                console.log(`[OK] Restored ${original}`);
                            }
                        } else {
                            const original = restoreChoice.replace('.bak', '');
                            fs.copyFileSync(path.join(cookedDir, restoreChoice), path.join(cookedDir, original));
                            fs.unlinkSync(path.join(cookedDir, restoreChoice));
                            console.log(`SUCCESS: ${original} restored.`);
                        }
                    }
                }
            } catch (e: any) {
                console.error(`Failed: ${e.message}`);
            }
            await inquirer.prompt([{ type: 'input', name: 'pause', message: 'Press Enter to continue...' }]);
            continue;
        }
    }
}

async function promptForItemAndUPK(message: string, cookedDir: string) {
    const { searchTerm } = await inquirer.prompt([{
        type: 'input',
        name: 'searchTerm',
        message
    }]);

    const matches = searchAssets(searchTerm);
    if (matches.length === 0) {
        console.error(`No items found matching "${searchTerm}".`);
        return null;
    }

    const { selectedItem } = await inquirer.prompt([{
        type: 'rawlist',
        name: 'selectedItem',
        message: 'Select item:',
        choices: matches.map(m => ({ 
            name: `${m.name} [${m.packageName}]`, 
            value: m 
        }))
    }]);

    const result = await resolvePackagePath(selectedItem.productId.toString(), cookedDir);
    if (!result) return null;

    let finalUpkPath = '';
    if ('candidates' in result) {
        const { selected } = await inquirer.prompt([{
            type: 'rawlist',
            name: 'selected',
            message: 'Multiple matches found. Select one:',
            choices: result.candidates.slice(0, 50)
        }]);
        finalUpkPath = path.join(cookedDir, selected);
    } else {
        finalUpkPath = result.path;
    }

    return { path: finalUpkPath, item: selectedItem };
}

function backupFile(filePath: string) {
    const backupPath = `${filePath}.bak`;
    if (!fs.existsSync(backupPath)) {
        console.log(`Creating backup: ${path.basename(backupPath)}`);
        fs.copyFileSync(filePath, backupPath);
    }
}

program
  .command('search')
  .argument('<term>', 'Keyword to search for')
  .action((term) => {
    const results = searchAssets(term);
    console.table(results);
  });

process.on('SIGINT', () => {
    console.log('\nExiting VelocityRL.');
    process.exit(0);
});

async function runSafeWizard() {
    try {
        const config = loadConfig();
        if (config.cookedDir) (global as any).COOKED_DIR = config.cookedDir;

        await checkVersion();
        await ensurePythonDependencies();
        await runInteractiveWizard();
    } catch (e: any) {
        if (e.name === 'ExitPromptError') {
            console.log('\nGoodbye!');
        } else {
            console.error('\nAn unexpected error occurred:', e.message);
        }
        process.exit(0);
    }
}

if (process.argv.length <= 2) {
    runSafeWizard();
} else {
    program.parse(process.argv);
}
