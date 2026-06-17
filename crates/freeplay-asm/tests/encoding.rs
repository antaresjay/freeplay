use std::collections::HashMap;

use freeplay_asm::{assemble, Bits};

fn asm32(source: &str) -> Vec<u8> {
    assemble(source, 0x1000, Bits::X86, &HashMap::new())
        .unwrap_or_else(|e| panic!("{source:?}: {e}"))
        .bytes
}

fn asm64(source: &str) -> Vec<u8> {
    assemble(source, 0x1000, Bits::X64, &HashMap::new())
        .unwrap_or_else(|e| panic!("{source:?}: {e}"))
        .bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn check(source: &str, expected: &str) {
    let got = hex(&asm32(source));
    assert_eq!(got, expected, "{source}");
}

#[test]
fn encodes_the_witcher_2_injection_points() {
    check("mov edx,[eax]", "8B 10");
    check("mov ecx,eax", "8B C8");
    check("call dword ptr [edx+00000234]", "FF 92 34 02 00 00");
    check("mov eax,[esi+08]", "8B 46 08");
    check("mov eax,[ebp+0C]", "8B 45 0C");
    check("mov [eax],ecx", "89 08");
    check("lea eax,[ebp-10]", "8D 45 F0");
    check("lea ecx,[edi+ecx+30]", "8D 4C 0F 30");
    check("cmp [edi+ecx+30],eax", "39 44 0F 30");
    check("add esp,0C", "83 C4 0C");
    check("and dword ptr [ebp-04],00", "83 65 FC 00");
    check("and dword ptr [eax],00", "83 20 00");
    check("cmp byte ptr [edi+00000138],00", "80 BF 38 01 00 00 00");
    check("test eax,eax", "85 C0");
    check("test al,al", "84 C0");
    check("mov al,01", "B0 01");
    check("inc eax", "40");
    check("inc [esi+18]", "FF 46 18");
    check("push esi", "56");
    check("pop ebp", "5D");
    check("leave", "C9");
    check("ret 0008", "C2 08 00");
    check("ret", "C3");
}

#[test]
fn encodes_the_x87_the_health_hook_uses() {
    check("fld dword ptr [eax]", "D9 00");
    check("fstp dword ptr [eax]", "D9 18");
    check("fld [eax+4]", "D9 40 04");
    check("fild [ebp+C]", "DB 45 0C");
    check("fistp [ebp+C]", "DB 5D 0C");
    check("fld1", "D9 E8");
    check("fldz", "D9 EE");
}

#[test]
fn encodes_the_sse_around_those_hooks() {
    check("movss xmm0,[eax]", "F3 0F 10 00");
    check("movss [esp+14],xmm0", "F3 0F 11 44 24 14");
    check("movsd [esp],xmm0", "F2 0F 11 04 24");
    check("ucomiss xmm0,[eax]", "0F 2E 00");
    check("cvtps2pd xmm0,xmm0", "0F 5A C0");
    check("mulss xmm0,xmm1", "F3 0F 59 C1");
}

#[test]
fn a_jump_to_a_label_is_five_bytes_so_a_hook_fits() {
    let out = asm32("start:\njmp done\nnop\ndone:\nret");
    assert_eq!(hex(&out), "E9 01 00 00 00 90 C3");
}

#[test]
fn a_backward_jump_counts_from_the_end_of_the_instruction() {
    let out = asm32("here:\nnop\njmp here");
    assert_eq!(hex(&out), "90 E9 FA FF FF FF");
}

#[test]
fn conditional_jumps_are_the_near_form() {
    assert_eq!(hex(&asm32("je 0")).len(), hex(&[0u8; 6]).len());
    let out = asm32("jne done\ndone:\nret");
    assert_eq!(hex(&out), "0F 85 00 00 00 00 C3");
}

#[test]
fn numbers_without_a_prefix_are_hex() {
    check("mov eax,10", "B8 10 00 00 00");
    check("mov eax,#10", "B8 0A 00 00 00");
}

#[test]
fn data_directives_lay_out_bytes() {
    assert_eq!(hex(&asm32("db 8B 10 8B C8")), "8B 10 8B C8");
    assert_eq!(hex(&asm32("dd 0")), "00 00 00 00");
    assert_eq!(hex(&asm32("dd (float)1")), "00 00 80 3F");
    assert_eq!(hex(&asm32("dq (double)1")), "00 00 00 00 00 00 F0 3F");
}

#[test]
fn nop_with_a_count_pads() {
    assert_eq!(hex(&asm32("nop 5")), "90 90 90 90 90");
}

#[test]
fn an_absolute_symbol_becomes_a_plain_address_in_32_bit() {
    let mut symbols = HashMap::new();
    symbols.insert("baseWitcher".to_string(), 0x0040_0000u64);
    let out = assemble("mov [baseWitcher],eax", 0x1000, Bits::X86, &symbols)
        .unwrap()
        .bytes;
    assert_eq!(hex(&out), "89 05 00 00 40 00");
}

#[test]
fn the_same_symbol_goes_rip_relative_in_64_bit() {
    let mut symbols = HashMap::new();
    symbols.insert("slot".to_string(), 0x1_0000_0100u64);
    let out = assemble("mov [slot],rax", 0x1_0000_0000, Bits::X64, &symbols)
        .unwrap()
        .bytes;
    assert_eq!(hex(&out), "48 89 05 F9 00 00 00");
}

#[test]
fn sixty_four_bit_registers_get_a_rex_prefix() {
    assert_eq!(hex(&asm64("mov rax,rbx")), "48 8B C3");
    assert_eq!(hex(&asm64("mov r8,r9")), "4D 8B C1");
    assert_eq!(hex(&asm64("push r12")), "41 54");
    assert_eq!(
        hex(&asm64("mov rax,1122334455")),
        "48 B8 55 44 33 22 11 00 00 00"
    );
}

#[test]
fn an_undefined_symbol_is_an_error_rather_than_zero() {
    let out = assemble("jmp nowhere", 0x1000, Bits::X86, &HashMap::new());
    assert!(out.is_err());
}

#[test]
fn a_label_defined_twice_is_refused() {
    let out = assemble("a:\nnop\na:\nnop", 0x1000, Bits::X86, &HashMap::new());
    assert!(out.is_err());
}

#[test]
fn comments_and_blank_lines_are_ignored() {
    let out = asm32("// leading\n\nnop // trailing\n; whole line\nret");
    assert_eq!(hex(&out), "90 C3");
}

#[test]
fn a_label_can_share_a_line_with_an_instruction() {
    let out = asm32("here: nop\njmp here");
    assert_eq!(hex(&out), "90 E9 FA FF FF FF");
}
