import * as fs from 'fs';
import * as path from 'path';
import * as zlib from 'zlib';
import * as crypto from 'crypto';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));

export class DecryptionProvider {
    private static keys: Buffer[] = [];

    static loadKeys() {
        if (this.keys.length > 0) return;
        const keysPath = path.join(__dirname, '../python/keys.txt');
        if (!fs.existsSync(keysPath)) {
            console.warn(" Warning: keys.txt not found. Encrypted files will fail.");
            return;
        }
        const lines = fs.readFileSync(keysPath, 'utf8').split('\n');
        this.keys = lines.map(l => Buffer.from(l.trim(), 'base64')).filter(b => b.length === 32);
    }

    static decryptECB(data: Buffer, key: Buffer): Buffer {
        const decipher = crypto.createDecipheriv('aes-256-ecb', key, null);
        decipher.setAutoPadding(false);
        return Buffer.concat([decipher.update(data), decipher.final()]);
    }

    static encryptECB(data: Buffer, key: Buffer): Buffer {
        const cipher = crypto.createCipheriv('aes-256-ecb', key, null);
        cipher.setAutoPadding(false);
        return Buffer.concat([cipher.update(data), cipher.final()]);
    }

    static findKey(encryptedHeader: Buffer, firstPlainByte: number): Buffer | null {
        this.loadKeys();
        for (const key of this.keys) {
            try {
                const decrypted = this.decryptECB(encryptedHeader.subarray(0, 16), key);
                // Check if the decrypted content makes sense. 
                // Usually the first entry in the name table is an FString.
                // If it's a valid string length, it's likely the right key.
                const len = decrypted.readInt32LE(0);
                if (Math.abs(len) < 1000 && Math.abs(len) > 0) {
                    return key;
                }
            } catch (e) {}
        }
        return null;
    }
}

export interface FCompressedChunk {
    uncompressedOffset: number;
    uncompressedSize: number;
    compressedOffset: number;
    compressedSize: number;
}

export interface UPKSummary {
    tag: number;
    fileVersion: number;
    licenseeVersion: number;
    headerSize: number;
    folderName: string;
    packageFlags: number;
    nameCount: number;
    nameOffset: number;
    exportCount: number;
    exportOffset: number;
    importCount: number;
    importOffset: number;
    dependsOffset: number;
    compressionFlags: number;
    compressedChunks: FCompressedChunk[];
}

export interface UPKExport {
    classIndex: number;
    superIndex: number;
    outerIndex: number;
    objectNameIndex: number;
    archetypeIndex: number;
    objectFlags: bigint;
    serialSize: number;
    serialOffset: number;
    exportFlags: number;
}

export class BinaryReader {
    public pos: number = 0;
    constructor(private buffer: Buffer) {}
    readI32(): number { const v = this.buffer.readInt32LE(this.pos); this.pos += 4; return v; }
    readU32(): number { const v = this.buffer.readUInt32LE(this.pos); this.pos += 4; return v; }
    readI64(): bigint { const v = this.buffer.readBigInt64LE(this.pos); this.pos += 8; return v; }
    readU64(): bigint { const v = this.buffer.readBigUint64LE(this.pos); this.pos += 8; return v; }
    readFString(): string {
        const len = this.readI32();
        if (len === 0) return "";
        if (len > 0) {
            const s = this.buffer.toString('utf8', this.pos, this.pos + len - 1);
            this.pos += len;
            return s;
        } else {
            const absLen = Math.abs(len) * 2;
            const s = this.buffer.toString('utf16le', this.pos, this.pos + absLen - 2);
            this.pos += absLen;
            return s;
        }
    }
    skip(n: number) { this.pos += n; }
}

export class UPKFile {
    public summary: UPKSummary | null = null;
    public exports: UPKExport[] = [];
    private dataBuffer: Buffer | null = null;
    private decryptionKey: Buffer | null = null;

    constructor(public filePath: string) {}

    public getData(): Buffer | null {
        return this.dataBuffer;
    }

    private readBytes(pos: number, size: number): Buffer {
        if (!this.dataBuffer) {
             const buffer = Buffer.alloc(size);
             const fd = fs.openSync(this.filePath, 'r');
             try {
                 fs.readSync(fd, buffer, 0, size, pos);
             } finally {
                 fs.closeSync(fd);
             }
             return buffer;
        }
        return this.dataBuffer.subarray(pos, pos + size);
    }

    readSummary() {
        const initialBuffer = this.readBytes(0, 4096); 
        const r = new BinaryReader(initialBuffer);

        const tag = r.readU32();
        if (tag !== 0x9E2A83C1) throw new Error('Invalid UPK Tag');

        const version = r.readU32();
        const fileVersion = version & 0xFFFF;
        const licenseeVersion = version >> 16;
        const headerSize = r.readI32();
        const folderName = r.readFString();
        const packageFlags = r.readU32();
        const nameCount = r.readI32();
        const nameOffset = r.readI32();
        const exportCount = r.readI32();
        const exportOffset = r.readI32();
        const importCount = r.readI32();
        const importOffset = r.readI32();
        const dependsOffset = r.readI32();
        // Rocket League encrypts the metadata of all compressed packages
        const isCompressed = (packageFlags & 0x02000000) !== 0;
        const isEncrypted = isCompressed || (packageFlags & 0x01000000) !== 0;

        let workingHeader = Buffer.alloc(headerSize);
        if (isEncrypted) {
            console.log(` Package is encrypted. Searching for key...`);
            const encryptedHeader = Buffer.alloc(headerSize);
            const fd = fs.openSync(this.filePath, 'r');
            fs.readSync(fd, encryptedHeader, 0, headerSize, 0);
            fs.closeSync(fd);

            let encryptedPart = encryptedHeader.subarray(nameOffset);
            const remainder = encryptedPart.length % 16;
            if (remainder !== 0) {
                const padded = Buffer.alloc(encryptedPart.length + (16 - remainder));
                encryptedPart.copy(padded);
                encryptedPart = padded;
            }
            const key = DecryptionProvider.findKey(encryptedPart, 0);
            if (!key) throw new Error("Could not find a valid decryption key for this package.");
            
            console.log(` Decryption key found: ${key.toString('base64').substring(0, 8)}...`);
            this.decryptionKey = key;
            const decryptedPart = DecryptionProvider.decryptECB(encryptedPart, key);
            
            encryptedHeader.copy(workingHeader, 0, 0, nameOffset);
            decryptedPart.copy(workingHeader, nameOffset);
        } else {
            const fd = fs.openSync(this.filePath, 'r');
            fs.readSync(fd, workingHeader, 0, headerSize, 0);
            fs.closeSync(fd);
        }

        // Use workingHeader for deeper parsing
        const hr = new BinaryReader(workingHeader);
        hr.pos = r.pos; // Sync positions

        hr.skip(16); // ImportExportGuidsOffset to ThumbnailTableOffset
        hr.skip(16); // Guid

        const genCount = hr.readI32();
        hr.skip(genCount * 12);
        hr.skip(8); // EngineVersion, CookerVersion
        
        const compressionFlags = hr.readU32();
        
        // Standard UE3 chunks
        const standardChunks: FCompressedChunk[] = [];
        const standardChunkCount = hr.readI32();
        for (let i = 0; i < standardChunkCount; i++) {
            standardChunks.push({
                uncompressedOffset: hr.readI32(),
                uncompressedSize: hr.readI32(),
                compressedOffset: hr.readI32(),
                compressedSize: hr.readI32()
            });
        }

        // Rocket League Metadata
        hr.readI32(); // Unknown
        const stringCount = hr.readI32();
        for (let i = 0; i < stringCount; i++) hr.readFString();
        
        const texAllocCount = hr.readI32();
        for (let i = 0; i < texAllocCount; i++) {
            hr.skip(20);
            const subCount = hr.readI32();
            hr.skip(subCount * 4);
        }

        // File Compression Metadata
        const garbageSize = hr.readI32();
        const rlChunksOffset = hr.readI32();
        const lastBlockSize = hr.readI32();

        let chunks = standardChunks;
        if (rlChunksOffset > 0) {
            const cr = new BinaryReader(workingHeader);
            cr.pos = nameOffset + rlChunksOffset;
            const rlChunkCount = cr.readI32();
            console.log(` RL Chunk Table found at 0x${cr.pos.toString(16)}. Count: ${rlChunkCount}`);
            
            if (rlChunkCount > 0 && rlChunkCount < 10000) {
                chunks = [];
                for (let i = 0; i < rlChunkCount; i++) {
                    chunks.push({
                        uncompressedOffset: Number(cr.readI64()),
                        uncompressedSize: cr.readI32(),
                        compressedOffset: Number(cr.readI64()),
                        compressedSize: cr.readI32()
                    });
                }
            }
        }

        this.summary = {
            tag, fileVersion, licenseeVersion, headerSize, folderName,
            packageFlags, nameCount, nameOffset, exportCount, exportOffset,
            importCount, importOffset, dependsOffset,
            compressionFlags, compressedChunks: chunks
        };

        if (isCompressed) {
            this.decompress(workingHeader);
        } else {
            this.dataBuffer = fs.readFileSync(this.filePath);
            if (isEncrypted) {
                workingHeader.copy(this.dataBuffer, 0, 0, headerSize);
                this.summary.packageFlags &= ~0x01000000;
            }
        }
    }

    private decompress(headerBuffer: Buffer) {
        if (!this.summary) return;
        if (this.summary.compressedChunks.length === 0) {
            console.warn(" Warning: Package marked as compressed but no chunks found.");
            this.dataBuffer = fs.readFileSync(this.filePath);
            return;
        }
        
        console.log(` Decompressing ${this.summary.compressedChunks.length} chunks...`);
        const fd = fs.openSync(this.filePath, 'r');
        let uncompressedBuffers = [headerBuffer];

        for (const chunk of this.summary.compressedChunks) {
            const chunkPayload = Buffer.alloc(chunk.compressedSize);
            fs.readSync(fd, chunkPayload, 0, chunk.compressedSize, chunk.compressedOffset);

            let payloadPos = 0;
            const magic = chunkPayload.readUInt32LE(payloadPos); payloadPos += 4;
            if (magic !== 0x9E2A83C1) {
                throw new Error(`Invalid chunk magic at offset ${chunk.compressedOffset}`);
            }

            const blockSize = chunkPayload.readInt32LE(payloadPos); payloadPos += 4;
            const totalCompressedSize = chunkPayload.readInt32LE(payloadPos); payloadPos += 4;
            const totalUncompressedSize = chunkPayload.readInt32LE(payloadPos); payloadPos += 4;

            const blocks: { compSize: number, uncompSize: number }[] = [];
            let sumUncomp = 0;
            while (sumUncomp < totalUncompressedSize) {
                const compSize = chunkPayload.readInt32LE(payloadPos); payloadPos += 4;
                const uncompSize = chunkPayload.readInt32LE(payloadPos); payloadPos += 4;
                blocks.push({ compSize, uncompSize });
                sumUncomp += uncompSize;
            }

            for (const block of blocks) {
                const compressedBlock = chunkPayload.subarray(payloadPos, payloadPos + block.compSize);
                payloadPos += block.compSize;
                
                try {
                    const inflated = zlib.inflateSync(compressedBlock);
                    uncompressedBuffers.push(inflated);
                } catch (e) {
                    throw new Error(`Block decompression failed in chunk at ${chunk.compressedOffset}: ${e}`);
                }
            }
        }
        fs.closeSync(fd);

        this.dataBuffer = Buffer.concat(uncompressedBuffers);
        this.summary.packageFlags &= ~0x02000000;
        console.log(` Decompression complete. Final size: ${this.dataBuffer.length} bytes.`);
    }

    private readFString(buffer: Buffer, pos: number): { str: string, bytesRead: number } {
        const length = buffer.readInt32LE(pos);
        if (length === 0) return { str: '', bytesRead: 4 };
        
        if (length > 0) {
            const str = buffer.toString('utf8', pos + 4, pos + 4 + length - 1);
            return { str, bytesRead: 4 + length };
        } else {
            const absLen = Math.abs(length) * 2;
            const str = buffer.toString('utf16le', pos + 4, pos + 4 + absLen - 2);
            return { str, bytesRead: 4 + absLen };
        }
    }

    readExportMap() {
        if (!this.summary) return;
        if (!this.dataBuffer) {
            console.error(" Error: Attempted to read ExportMap before data buffer was loaded.");
            return;
        }
        
        const r = new BinaryReader(this.dataBuffer);
        r.pos = this.summary.exportOffset;
        this.exports = [];

        console.log(` Parsing Export Map (${this.summary.exportCount} entries)...`);
        for (let i = 0; i < this.summary.exportCount; i++) {
            const classIndex = r.readI32();
            const superIndex = r.readI32();
            const outerIndex = r.readI32();
            const objectNameIndex = r.readI32();
            const objectNameNumber = r.readI32(); // FName instance number
            const archetypeIndex = r.readI32();
            const objectFlags = r.readU64();
            const serialSize = r.readI32();
            const serialOffset = Number(r.readI64());
            const exportFlags = r.readI32();
            
            // Rocket League extras
            const netObjCount = r.readI32();
            r.skip(netObjCount * 4);
            r.skip(16); // PackageGuid
            r.readI32(); // PackageFlags

            this.exports.push({
                classIndex, superIndex, outerIndex,
                objectNameIndex, archetypeIndex,
                objectFlags, serialSize, serialOffset, exportFlags
            });
        }
    }

    extractExport(index: number): Buffer {
        if (this.exports.length === 0) throw new Error('Export table is empty. Did readExportMap fail?');
        const exp = this.exports[index];
        if (!exp) throw new Error(`Export index ${index} out of bounds (Table size: ${this.exports.length})`);
        return this.readBytes(exp.serialOffset, exp.serialSize);
    }

    save() {
        if (!this.dataBuffer || !this.summary) throw new Error('File not loaded');

        // If we have a key, we MUST re-encrypt the header before saving
        if (this.decryptionKey) {
            console.log(` Re-encrypting header for ${path.basename(this.filePath)}...`);
            const headerSize = this.summary.headerSize;
            const nameOffset = this.summary.nameOffset;
            
            let plainPart = this.dataBuffer.subarray(nameOffset, headerSize);
            const remainder = plainPart.length % 16;
            if (remainder !== 0) {
                const padded = Buffer.alloc(plainPart.length + (16 - remainder));
                plainPart.copy(padded);
                plainPart = padded;
            }
            const encryptedPart = DecryptionProvider.encryptECB(plainPart, this.decryptionKey);
            
            const finalSaveBuffer = Buffer.from(this.dataBuffer);
            encryptedPart.copy(finalSaveBuffer, nameOffset, 0, headerSize - nameOffset);
            
            // Set the encrypted flag back
            finalSaveBuffer.writeUInt32LE(this.summary.packageFlags | 0x01000000, this.findPackageFlagsOffset());
            
            fs.writeFileSync(this.filePath, finalSaveBuffer);
        } else {
            fs.writeFileSync(this.filePath, this.dataBuffer);
        }
        
        console.log(` Patched and Saved: ${path.basename(this.filePath)}`);
    }

    patchExport(index: number, newContent: Buffer) {
        const exp = this.exports[index];
        if (!exp) throw new Error('Export index out of bounds');
        if (!this.dataBuffer || !this.summary) throw new Error('File not loaded');

        const patchSize = Math.min(newContent.length, exp.serialSize);
        newContent.copy(this.dataBuffer, exp.serialOffset, 0, patchSize);
        
        this.save();
    }

    private findPackageFlagsOffset(): number {
        // Basic summary: tag(4), version(4), headerSize(4), folderName(var)
        // packageFlags comes after folderName
        // folderName is an FString
        const folderNameLen = this.dataBuffer!.readInt32LE(12);
        const bytesRead = folderNameLen === 0 ? 4 : (folderNameLen > 0 ? 4 + folderNameLen : 4 + Math.abs(folderNameLen) * 2);
        return 12 + bytesRead;
    }
}
