import fs from 'fs';
import { UPKFile } from '../dist/upk.js';

const upk = new UPKFile('E:\\games\\rocketleague\\TAGame\\CookedPCConsole\\explosion_badaboom_SF.upk');
async function run() {
    try {
        upk.readSummary();
        
        const { BinaryReader } = await import('../dist/upk.js');
        const r = upk.dataBuffer ? new BinaryReader(upk.dataBuffer) : null;
        if (r) {
            r.pos = upk.summary.exportOffset;
        for (let i = 0; i < upk.summary.exportCount; i++) {
            const classIndex = r.readI32();
            const superIndex = r.readI32();
            const outerIndex = r.readI32();
            const objectNameIndex = r.readI32();
            const objectNameNumber = r.readI32();
            const archetypeIndex = r.readI32();
            const objectFlags = r.readU64();
            const serialSize = r.readI32();
            const serialOffset = r.readI32();
            const exportFlags = r.readI32();
            
            const netObjCount = r.readI32();
            console.log(`Export ${i}: NetObjCount = ${netObjCount}, pos before skip: ${r.pos}`);
            if (netObjCount < 0 || netObjCount > 1000) {
                console.log(`Abnormal netObjCount: ${netObjCount} at export ${i}`);
                break;
            }
            for (let j=0; j<netObjCount; j++) {
                r.readI32();
            }
            r.skip(16); // PackageGuid
            r.readI32(); // PackageFlags
        }
    }
} catch (e) {
    console.error(e);
}
}
run();
