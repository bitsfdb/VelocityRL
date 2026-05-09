import * as fs from 'fs';

const filePath = 'E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk';

class BinaryReader {
    constructor(buffer) { this.buffer = buffer; this.pos = 0; }
    readI32() { const v = this.buffer.readInt32LE(this.pos); this.pos += 4; return v; }
    readU32() { const v = this.buffer.readUInt32LE(this.pos); this.pos += 4; return v; }
    readFString() {
        const len = this.readI32();
        if (len === 0) return "";
        this.pos += len;
        return "str";
    }
    skip(n) { this.pos += n; }
}

const fd = fs.openSync(filePath, 'r');
const prefix = Buffer.alloc(67077);
fs.readSync(fd, prefix, 0, 67077, 0);

const r = new BinaryReader(prefix);
r.skip(12); // Tag, Version, HeaderSize
r.readFString(); // FolderName
r.skip(40); // Flags to DependsOffset
r.skip(32); // Guids/Guid

const genCount = r.readI32();
r.skip(genCount * 12);
r.skip(12); // Engine, Cooker, CompFlags

const standardChunkCount = r.readI32();
r.skip(standardChunkCount * 16);

r.readI32(); // Unknown
const stringCount = r.readI32();
for (let i = 0; i < stringCount; i++) r.readFString();

const texAllocCount = r.readI32();
console.log('TexAllocCount:', texAllocCount);
for (let i = 0; i < texAllocCount; i++) {
    r.readI32();
    r.readI32();
    r.readI32();
    r.readI32();
    r.readI32();
    const subCount = r.readI32();
    r.skip(subCount * 4);
}

console.log('Pos before metadata:', r.pos.toString(16));
const garbageSize = r.readI32();
console.log('GarbageSize:', garbageSize);
const rlChunksOffset = r.readI32();
console.log('RLChunksOffset:', rlChunksOffset);
const lastBlockSize = r.readI32();
console.log('LastBlockSize:', lastBlockSize);

fs.closeSync(fd);
