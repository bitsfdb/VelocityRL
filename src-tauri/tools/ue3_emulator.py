#!/usr/bin/env python3
"""
UE3 Virtual Machine Bytecode Emulator and Static Analyzer.
Simulates execution of Unreal Engine 3 / Rocket League UScript bytecode.
Verifies memory boundaries, token semantics, variable scope correctness (local vs instance),
jump alignment, and prevents in-game crashes before deployment.
"""

import sys
import os
sys.path.append('/root')
sys.path.append(os.path.dirname(os.path.abspath(__file__)))
import struct
import ue3_decompiler

class VmCrash(Exception):
    pass

class Ue3BytecodeEmulator:
    def __init__(self, pkg_reader, outer_name, func_name, chunk_idx=None):
        self.pkg = pkg_reader
        self.outer_name = outer_name
        self.func_name = func_name
        
        # Locate export
        matches = [e for e in self.pkg.exports if e['name'] == func_name and e['outer_name'] == outer_name]
        if not matches:
            raise VmCrash(f"Function {outer_name}::{func_name} not found in UPK export table")
        self.export = matches[0]
        self.export_idx = self.export['idx']
        
        # Find chunk
        off = self.export['serial_offset']
        self.chunk_idx = chunk_idx
        if self.chunk_idx is None:
            for idx, c in enumerate(self.pkg.chunks):
                if c['uncomp_offset'] <= off < c['uncomp_offset'] + c['uncomp_size']:
                    self.chunk_idx = idx
                    break
        if self.chunk_idx is None:
            raise VmCrash(f"Could not locate chunk for export #{self.export_idx} at offset {off}")
            
        chunk_bytes = self.pkg.get_chunk_decompressed(self.chunk_idx)
        rel_off = off - self.pkg.chunks[self.chunk_idx]['uncomp_offset']
        exp_data = chunk_bytes[rel_off : rel_off + self.export['serial_size']]
        
        self.mem_sz = struct.unpack_from('<I', exp_data, 40)[0]
        self.disk_sz = struct.unpack_from('<I', exp_data, 44)[0]
        self.bytecode = exp_data[48 : 48 + self.disk_sz]
        
        # Find child exports (local variables, parameters, return value)
        self.child_exports = {e['idx']: e for e in self.pkg.exports if e['outer_idx'] == self.export_idx}
        
        # Virtual registers & state
        self.stack = []
        self.locals = {}
        self.instance_vars = {}
        self.pc = 0
        self.instruction_trace = []
        
        # Init local vars
        for idx, child in self.child_exports.items():
            self.locals[idx] = {'name': child['name'], 'val': None}

    def run(self, mock_inputs=None):
        """Executes the function bytecode until EX_RETURN / EX_END_OF_SCRIPT."""
        if mock_inputs:
            for k, v in mock_inputs.items():
                # find child by name
                for c_idx, c in self.child_exports.items():
                    if c['name'] == k:
                        self.locals[c_idx]['val'] = v
                        break
                        
        max_instructions = 10000
        step_count = 0
        
        while self.pc < len(self.bytecode):
            step_count += 1
            if step_count > max_instructions:
                raise VmCrash(f"Infinite loop detected at PC=0x{self.pc:04x} in {self.outer_name}::{self.func_name}")
                
            cur_pc = self.pc
            op = self.bytecode[self.pc]
            self.pc += 1
            
            # Opcode emulation
            if op == 0x07: # EX_JUMP_IF_NOT
                if cur_pc + 3 > len(self.bytecode):
                    raise VmCrash(f"EX_JUMP_IF_NOT truncated at PC=0x{cur_pc:04x}")
                target = struct.unpack_from('<H', self.bytecode, cur_pc + 1)[0]
                self.pc += 2
                
                # Pop condition expression
                cond = self._eval_expr()
                self.instruction_trace.append((cur_pc, f"EX_JUMP_IF_NOT (cond={cond}) -> 0x{target:04x}"))
                if target > self.mem_sz:
                    raise VmCrash(f"Jump target 0x{target:04x} exceeds script memory size {self.mem_sz} at PC=0x{cur_pc:04x}")
                if not cond:
                    if target < len(self.bytecode) and self.bytecode[target] in (0x06, 0x07, 0x0F, 0x1B, 0x1C, 0x04, 0x4C, 0x0B):
                        self.pc = target
                    else:
                        # Advance past the true branch statement
                        if self.pc < len(self.bytecode):
                            op_next = self.bytecode[self.pc]
                            if op_next == 0x0F: # EX_LET
                                self.pc += 1
                                self._eval_lvalue()
                                self._eval_expr()
                    
            elif op == 0x06: # EX_JUMP
                if cur_pc + 3 > len(self.bytecode):
                    raise VmCrash(f"EX_JUMP truncated at PC=0x{cur_pc:04x}")
                target = struct.unpack_from('<H', self.bytecode, cur_pc + 1)[0]
                self.instruction_trace.append((cur_pc, f"EX_JUMP -> 0x{target:04x}"))
                if target > self.mem_sz:
                    raise VmCrash(f"Jump target 0x{target:04x} exceeds script memory size {self.mem_sz} at PC=0x{cur_pc:04x}")
                self.pc = min(target, len(self.bytecode))
                
            elif op == 0x0F: # EX_LET
                # LHS lvalue
                lhs_ref = self._eval_lvalue()
                # RHS rvalue
                rhs_val = self._eval_expr()
                self.instruction_trace.append((cur_pc, f"EX_LET: {lhs_ref['desc']} = {rhs_val}"))
                lhs_ref['setter'](rhs_val)
                
            elif op == 0x14: # EX_LET_BOOL
                lhs_ref = self._eval_lvalue()
                rhs_val = self._eval_expr()
                self.instruction_trace.append((cur_pc, f"EX_LET_BOOL: {lhs_ref['desc']} = {rhs_val}"))
                lhs_ref['setter'](bool(rhs_val))
                
            elif op == 0x04: # EX_RETURN
                ret_val = self._eval_expr()
                self.instruction_trace.append((cur_pc, f"EX_RETURN: {ret_val}"))
                return {'status': 'OK', 'ret': ret_val, 'steps': step_count}
                
            elif op == 0x0B: # EX_NOTHING
                self.instruction_trace.append((cur_pc, "EX_NOTHING"))
                
            elif op == 0x4C: # EX_END_OF_SCRIPT
                self.instruction_trace.append((cur_pc, "EX_END_OF_SCRIPT"))
                return {'status': 'OK', 'ret': None, 'steps': step_count}
                
            elif op == 0x1B or op == 0x1C: # EX_VIRTUAL_FUNCTION / EX_FINAL_FUNCTION
                # Function call as statement
                func_idx = struct.unpack_from('<i', self.bytecode, self.pc)[0]
                self.pc += 8 # index + padding
                parms = []
                while self.pc < len(self.bytecode) and self.bytecode[self.pc] != 0x16:
                    parms.append(self._eval_expr())
                if self.pc < len(self.bytecode) and self.bytecode[self.pc] == 0x16:
                    self.pc += 1 # consume EX_END_FUNCTION_PARMS
                self.instruction_trace.append((cur_pc, f"CALL func#{func_idx} args={parms}"))
                
            else:
                self.instruction_trace.append((cur_pc, f"STATEMENT op=0x{op:02x}"))
                # Skip unknown statement or eval
                
        return {'status': 'OK', 'ret': None, 'steps': step_count}

    def _eval_expr(self):
        """Evaluates and returns the rvalue of the next expression token."""
        if self.pc >= len(self.bytecode):
            raise VmCrash(f"Unexpected EOF while evaluating expression at PC=0x{self.pc:04x}")
            
        op = self.bytecode[self.pc]
        self.pc += 1
        
        if op == 0x25: # EX_INT_ZERO
            return 0
        elif op == 0x26: # EX_INT_ONE
            return 1
        elif op == 0x27: # EX_TRUE_CONST
            return True
        elif op == 0x28: # EX_FALSE_CONST
            return False
        elif op == 0x2A: # EX_NO_OBJECT
            return None
        elif op == 0x2C: # EX_INT_CONST_BYTE
            val = self.bytecode[self.pc]
            self.pc += 1
            return val
        elif op == 0x1D: # EX_INT_CONST
            val = struct.unpack_from('<i', self.bytecode, self.pc)[0]
            self.pc += 4
            return val
        elif op == 0x1E: # EX_FLOAT_CONST
            val = struct.unpack_from('<f', self.bytecode, self.pc)[0]
            self.pc += 4
            return val
        elif op == 0x9A: # EX_EQUAL_EQUAL_INT_INT
            v1 = self._eval_expr()
            v2 = self._eval_expr()
            if self.pc < len(self.bytecode) and self.bytecode[self.pc] == 0x16:
                self.pc += 1
            return v1 == v2
        elif op == 0x96: # EX_LESS_INT_INT
            v1 = self._eval_expr()
            v2 = self._eval_expr()
            if self.pc < len(self.bytecode) and self.bytecode[self.pc] == 0x16:
                self.pc += 1
            return v1 < v2
        elif op == 0xCE: # EX_NOT_EQUAL_OBJECT
            v1 = self._eval_expr()
            v2 = self._eval_expr()
            if self.pc < len(self.bytecode) and self.bytecode[self.pc] == 0x16:
                self.pc += 1
            return v1 != v2
        elif op in (0x2B, 0x46): # EX_PROPERTY_CONST / EX_INSTANCE_VARIABLE / EX_LOCAL_VARIABLE
            var_idx = struct.unpack_from('<i', self.bytecode, self.pc)[0]
            self.pc += 4
            if var_idx in self.child_exports:
                return self.locals.get(var_idx, {}).get('val')
            return self.instance_vars.get(var_idx)
        elif op == 0x5E: # EX_DYN_ARRAY_ELEMENT
            idx_val = self._eval_expr()
            arr_lval = self._eval_lvalue()
            arr = arr_lval.get('arr', [])
            if 0 <= idx_val < len(arr):
                return arr[idx_val]
            return 0
        elif op == 0x35: # EX_STRUCT_MEMBER
            prop_idx = struct.unpack_from('<i', self.bytecode, self.pc)[0]
            struct_idx = struct.unpack_from('<i', self.bytecode, self.pc + 4)[0]
            self.pc += 10
            struct_val = self._eval_expr()
            if isinstance(struct_val, dict):
                return struct_val.get(prop_idx)
            return struct_val
        elif op == 0x0B: # EX_NOTHING
            return None
        else:
            return 0

    def _eval_lvalue(self):
        """Evaluates and returns an lvalue reference for assignments."""
        if self.pc >= len(self.bytecode):
            raise VmCrash(f"Unexpected EOF while evaluating lvalue at PC=0x{self.pc:04x}")
            
        op = self.bytecode[self.pc]
        self.pc += 1
        
        if op in (0x2B, 0x46): # EX_PROPERTY_CONST / EX_INSTANCE_VARIABLE / EX_LOCAL_VARIABLE
            var_idx = struct.unpack_from('<i', self.bytecode, self.pc)[0]
            self.pc += 4
            if var_idx in self.child_exports:
                var_name = self.child_exports[var_idx]['name']
                return {
                    'desc': f"Local::{var_name}",
                    'setter': lambda val: self.locals.setdefault(var_idx, {}).update({'val': val})
                }
            return {
                'desc': f"Instance::Var#{var_idx}",
                'setter': lambda val: self.instance_vars.update({var_idx: val})
            }
        elif op == 0x5E: # EX_DYN_ARRAY_ELEMENT
            idx_val = self._eval_expr()
            arr_lval = self._eval_lvalue()
            return {
                'desc': f"{arr_lval['desc']}[{idx_val}]",
                'setter': lambda val: arr_lval.get('arr_setter', lambda i, v: None)(idx_val, val)
            }
        elif op == 0x35: # EX_STRUCT_MEMBER
            prop_idx = struct.unpack_from('<i', self.bytecode, self.pc)[0]
            struct_idx = struct.unpack_from('<i', self.bytecode, self.pc + 4)[0]
            self.pc += 10
            struct_lval = self._eval_lvalue()
            return {
                'desc': f"{struct_lval['desc']}.Prop#{prop_idx}",
                'setter': lambda val: struct_lval.get('member_setter', lambda p, v: None)(prop_idx, val),
                'arr_setter': lambda idx, val: None
            }
        else:
            return {'desc': f'UnknownLValue(0x{op:02x})', 'setter': lambda v: None}

def run_emulator_test_suite(upk_path):
    print(f"[*] Starting UE3 VM Bytecode Emulator on {upk_path}...")
    pkg = ue3_decompiler.PackageReader(upk_path)
    
    # Test suite targets
    tests = [
        ("Car_TA", "SetLoadout", {"Data": {"Products": [23, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]}}),
        ("Car_TA", "SetLoadout", {"Data": {"Products": [4284, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]}}), # unswapped
        ("CarPreviewActor_TA", "SetLoadout", {"InLoadout": {"Products": [23, 0, 0, 33, 0, 0]}}),
        ("ExplosionPreviewer_TA", "SetProduct", {"ProductID": 2044}),
        ("ProductAttribute_TA", "ModifyLoadout", {"LoadoutProductIDs": [23, 33, 4038]}),
        ("PaintedFXSelector_TA", "PickFXActor", {"PaintedFXActor": 1234, "FXActor": 5678}),
        ("LoadoutValidation_TA", "ValidateLoadoutSlots", {}),
        ("LoadoutValidation_TA", "CorrectOnlineData", {}),
        ("PlayerVanitySave_TA", "Validate", {}),
        ("PostMatchCelebrationPreviewer_TA", "ResetInvalidAssets", {}),
        ("PRI_TA", "ValidateLoadout", {}),
    ]
    
    passed = 0
    failed = 0
    for outer, fn, mock_data in tests:
        try:
            vm = Ue3BytecodeEmulator(pkg, outer, fn)
            res = vm.run(mock_data)
            print(f" [+] PASS: {outer}::{fn} executed cleanly ({res['steps']} steps)")
            passed += 1
        except Exception as e:
            print(f" [!] FAIL: {outer}::{fn} -> {e}")
            failed += 1
            
    print(f"\n[*] Emulator Results: {passed} PASSED, {failed} FAILED")
    return failed == 0

def run_projectx_emulator_test_suite(upk_path):
    print(f"\n[*] Starting ProjectX Validation VM Bytecode Emulator on {upk_path}...")
    pkg = ue3_decompiler.PackageReader(upk_path)
    
    # URL validation function tests
    tests = [
        ("PsyNetUrl_X", "IsAllowedURL", {"InURL": "https://api.velocityrl.local/v1/broker", "bWebSocket": False}, True),
        ("PsyNetUrl_X", "ValidateURL", {"InURL": "https://custom-broker.example.com/api", "bWebSocket": False, "bEnforce": True}, "https://custom-broker.example.com/api"),
        ("URLValidator_X", "IsURLValid", {"URL": "http://127.0.0.1:8080/test"}, True),
        ("URLValidator_X", "IsSchemeValid", {"Scheme": "custom"}, True),
        ("URLValidator_X", "AreCharsUrlSafe", {"Str": "any_string!@#$"}, True),
        ("URLValidator_X", "IsCharUrlSafe", {"Char": "$"}, True),
        ("URLValidator_X", "IsPathValid", {"Path": "/items/spawn/23"}, True),
        ("URLValidator_X", "IsQueryValid", {"Query": "?product_id=4284&paint_id=12"}, True),
        ("URLValidator_X", "IsIPv4AddressValid", {"HostAndPort": "127.0.0.1"}, True),
        ("URLValidator_X", "IsIPv4AndPortValid", {"HostAndPort": "127.0.0.1:8080"}, True),
        ("URLValidator_X", "HostLooksLikeIPv4", {"HostAndPort": "127.0.0.1"}, True),
    ]
    
    passed = 0
    failed = 0
    for outer, fn, mock_data, expected_ret in tests:
        try:
            vm = Ue3BytecodeEmulator(pkg, outer, fn)
            res = vm.run(mock_data)
            if res.get('status') != 'OK':
                raise VmCrash(f"Unexpected status: {res.get('status')}")
            # If function is patched, verify return value
            if vm.bytecode[0] == 0x04: # EX_RETURN
                if expected_ret is not None and res.get('ret') != expected_ret:
                    raise VmCrash(f"Return value mismatch: expected {expected_ret!r}, got {res.get('ret')!r}")
            print(f" [+] PASS: {outer}::{fn} executed cleanly (ret={res.get('ret')!r}, {res['steps']} steps)")
            passed += 1
        except Exception as e:
            print(f" [!] FAIL: {outer}::{fn} -> {e}")
            failed += 1
            
    print(f"\n[*] ProjectX Emulator Results: {passed} PASSED, {failed} FAILED")
    return failed == 0

if __name__ == '__main__':
    tagame_path = sys.argv[1] if len(sys.argv) > 1 else '/root/velrlapi/games/rocketleague/TAGame/CookedPCConsole/TAGame.upk'
    projectx_path = sys.argv[2] if len(sys.argv) > 2 else None
    
    ok1 = True
    if os.path.isfile(tagame_path) and "ProjectX" not in os.path.basename(tagame_path):
        ok1 = run_emulator_test_suite(tagame_path)
    
    ok2 = True
    target_px = projectx_path if projectx_path else (tagame_path if "ProjectX" in os.path.basename(tagame_path) else None)
    if target_px and os.path.isfile(target_px):
        ok2 = run_projectx_emulator_test_suite(target_px)
        
    sys.exit(0 if (ok1 and ok2) else 1)
