/**
 * VelocityRL — RLIM JavaScript Plugin SDK Type Definitions
 */

export interface BytecodeBuilder {
    byte(b: number): this;
    int32(val: number): this;
    float32(val: number): this;
    let(targetBuilder: (b: this) => void, valueBuilder: (b: this) => void): this;
    instanceVar(varIndex: number): this;
    localVar(varIndex: number): this;
    constFloat(val: number): this;
    constInt(val: number): this;
    constBool(val: boolean): this;
    call(funcIndex: number, ...argBuilders: Array<(b: this) => void>): this;
    return(exprBuilder?: ((b: this) => void) | null): this;
    padToSize(targetSize: number): this;
    toArray(): number[];
    toUint8Array(): Uint8Array;
}

export declare const BytecodeBuilder: {
    new (): BytecodeBuilder;
};

export declare const Opcodes: Record<string, number>;
export declare const FunctionFlags: Record<string, number>;

export interface CreateFunctionOptions {
    flags?: number;
    bytecode?: number[] | Uint8Array | BytecodeBuilder;
}

export interface RlimPatcher {
    patchFunction(packageName: string, className: string, funcName: string, bytecode: number[] | Uint8Array | BytecodeBuilder): boolean;
    createFunction(packageName: string, className: string, funcName: string, options?: CreateFunctionOptions): boolean;
    neutralize(packageName: string, className: string, funcName: string, returnBool?: boolean): boolean;
}

export interface RlimShaders {
    setCamber(frontDegrees: number, rearDegrees: number): boolean;
    setUnderglow(rgb?: [number, number, number], brightness?: number, mode?: number): boolean;
}

export interface RlimSpawner {
    spawnItem(productId: number, paintId?: number): boolean;
}

export interface RlimConfig {
    get(key: string, defaultValue?: string): string;
    set(key: string, value: string | number | boolean): boolean;
}

export interface RlimHost {
    patcher: RlimPatcher;
    shaders: RlimShaders;
    spawner: RlimSpawner;
    config: RlimConfig;
    log(msg: string): void;
}

export declare const rlim: RlimHost;
export default rlim;
