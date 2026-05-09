import * as fs from 'fs';
import { UPKFile } from './upk.js';

/**
 * Implements the "Fixed Rename" logic from RLUPKTools.
 * This renames a name table entry IN-PLACE by padding with nulls.
 * This ensures the header size and all subsequent offsets remain identical.
 */
export class UPKSwapper {
    constructor(private upk: UPKFile) {}

    /**
     * Renames an existing name entry to a new name.
     * The new name must be shorter than or equal to the original name's space.
     */
    fixedRename(oldName: string, newName: string): boolean {
        if (!this.upk.summary || !this.upk.getData()) return false;

        const data = this.upk.getData()!;
        const nameOffset = this.upk.summary.nameOffset;
        let pos = nameOffset;

        for (let i = 0; i < this.upk.summary.nameCount; i++) {
            const entryStart = pos;
            const length = data.readInt32LE(pos);
            pos += 4;

            let currentName: string;
            let totalBytes: number;

            if (length > 0) {
                // ANSI
                currentName = data.toString('utf8', pos, pos + length - 1);
                totalBytes = length;
                pos += length;
            } else if (length < 0) {
                // UTF-16
                const absLen = Math.abs(length) * 2;
                currentName = data.toString('utf16le', pos, pos + absLen - 2);
                totalBytes = absLen;
                pos += absLen;
            } else {
                pos += 8; // Skip flags
                continue;
            }

            // Skip ObjectFlags (8 bytes)
            pos += 8;

            if (currentName === oldName) {
                console.log(` Found name entry "${oldName}" at index ${i}. Renaming to "${newName}"...`);
                
                // Prepare the new string
                const newBuf = Buffer.alloc(totalBytes, 0);
                if (length > 0) {
                    newBuf.write(newName, 'utf8');
                } else {
                    newBuf.write(newName, 'utf16le');
                }

                // Write it back
                newBuf.copy(data, entryStart + 4);
                return true;
            }
        }

        return false;
    }

    /**
     * Swaps all references of donorName to targetName in the package.
     * This is useful for "tricking" the engine into loading one asset instead of another.
     */
    swapAssetReferences(targetAssetName: string, donorAssetName: string) {
        console.log(` Swapping references: ${targetAssetName} <-> ${donorAssetName}`);
        
        // We actually want to rename the TARGET name to something else,
        // and the DONOR name (which we might have imported) to the TARGET name.
        // But for a simple swap where we just patch the file, we can just rename 
        // the internal package name.
        
        const success = this.fixedRename(targetAssetName, donorAssetName);
        if (success) {
            console.log(` Successfully swapped ${targetAssetName} for ${donorAssetName} references.`);
        } else {
            console.warn(` Failed to find name entry for ${targetAssetName}.`);
        }
    }
}
