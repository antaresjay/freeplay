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

#[test]
fn the_avx_forms_carry_a_vex_prefix() {
    assert_eq!(hex(&asm64("vmovss xmm0,[rax]")), "C5 FA 10 00");
    assert_eq!(hex(&asm64("vmovss [rax],xmm0")), "C5 FA 11 00");
    assert_eq!(hex(&asm64("vaddss xmm0,xmm1,xmm2")), "C5 F2 58 C2");
    assert_eq!(hex(&asm64("vxorps xmm0,xmm0,xmm0")), "C5 F8 57 C0");
    // written with two operands the destination is the source as well
    assert_eq!(hex(&asm64("vmulss xmm1,xmm3")), "C5 F2 59 CB");
    // nothing to merge into, so vvvv stays unset
    assert_eq!(hex(&asm64("vcomiss xmm0,xmm1")), "C5 F8 2F C1");
    // the second half of the register file needs the longer prefix, unless
    // it is only the destination
    assert_eq!(hex(&asm64("vmovss xmm15,[rax]")), "C5 7A 10 38");
    assert_eq!(hex(&asm64("vmovaps xmm0,[r9]")), "C4 C1 78 28 01");
    assert_eq!(hex(&asm64("vmovups [rcx+44],ymm0")), "C5 FC 11 41 44");
}

#[test]
fn the_x87_stack_forms_pick_the_right_direction() {
    assert_eq!(hex(&asm32("faddp")), "DE C1");
    assert_eq!(hex(&asm32("fcompp")), "DE D9");
    assert_eq!(hex(&asm32("fcomip st(1)")), "DF F1");
    assert_eq!(hex(&asm32("fnstsw ax")), "DF E0");
    assert_eq!(hex(&asm32("fadd st(0),st(1)")), "D8 C1");
    assert_eq!(hex(&asm32("fadd st(1),st(0)")), "DC C1");
    assert_eq!(hex(&asm32("fiadd dword ptr [eax]")), "DA 00");
    assert_eq!(hex(&asm32("fnstcw word ptr [esp]")), "D9 3C 24");
}

#[test]
fn the_sse_gaps_encode() {
    assert_eq!(hex(&asm32("pxor xmm0,xmm0")), "66 0F EF C0");
    assert_eq!(hex(&asm32("shufps xmm0,xmm1,0")), "0F C6 C1 00");
    assert_eq!(hex(&asm32("maxsd xmm0,xmm1")), "F2 0F 5F C1");
    assert_eq!(hex(&asm32("psrld xmm2,1")), "66 0F 72 D2 01");
    assert_eq!(hex(&asm32("movmskps eax,xmm3")), "0F 50 C3");
    assert_eq!(hex(&asm32("cmpeqps xmm3,xmm3")), "0F C2 DB 00");
    assert_eq!(hex(&asm32("roundss xmm0,xmm1,3")), "66 0F 3A 0A C1 03");
    assert_eq!(hex(&asm32("movdqu dqword [esp],xmm5")), "F3 0F 7F 2C 24");
}

#[test]
fn the_rest_of_the_integer_set() {
    assert_eq!(hex(&asm32("rdtsc")), "0F 31");
    assert_eq!(hex(&asm32("bt eax,1")), "0F BA E0 01");
    assert_eq!(hex(&asm32("bts eax,ebx")), "0F AB D8");
    assert_eq!(hex(&asm32("bswap eax")), "0F C8");
    assert_eq!(hex(&asm32("rep movsd")), "F3 A5");
    assert_eq!(hex(&asm32("loop here\nhere:")), "E2 00");
    assert_eq!(hex(&asm32("xchg [esp+08],eax")), "87 44 24 08");
    // the two operand shorthand for the three operand multiply
    assert_eq!(hex(&asm64("imul rdx,4")), "48 6B D2 04");
}

#[test]
fn a_segment_override_goes_in_front() {
    assert_eq!(hex(&asm32("mov eax,fs:[10]")), "64 8B 05 10 00 00 00");
    assert_eq!(hex(&asm64("mov rax,gs:[rbx+8]")), "65 48 8B 43 08");
}

#[test]
fn cheat_engine_spellings_of_the_same_thing() {
    // pascal hex, a cast, a character tag and a decimal escape
    assert_eq!(hex(&asm32("mov eax,$10")), "B8 10 00 00 00");
    assert_eq!(hex(&asm32("mov eax,(int)100")), "B8 64 00 00 00");
    assert_eq!(hex(&asm32("mov eax,#100")), "B8 64 00 00 00");
    assert_eq!(hex(&asm32("mov eax,'Bott'")), "B8 42 6F 74 74");
    assert_eq!(hex(&asm64("mov r15l,1")), "41 B7 01");
    // a string in a db is bytes, not a number
    assert_eq!(hex(&asm32("db 'ab',00")), "61 62 00");
    // arithmetic the author left for the assembler
    assert_eq!(hex(&asm32("mov [edi+7*8+30],eax")), "89 47 68");
}

#[test]
fn a_wildcard_byte_is_left_alone_rather_than_written() {
    let out = assemble("db 90 ?? 90", 0x1000, Bits::X86, &HashMap::new()).unwrap();
    assert_eq!(out.bytes, vec![0x90, 0x00, 0x90]);
    assert_eq!(out.holes, vec![1]);
}

#[test]
fn an_at_f_with_no_anonymous_label_lands_on_the_next_one() {
    let out = asm32("cmp eax,1\njge @f\nnop\nend:\nret");
    assert_eq!(hex(&out), "83 F8 01 0F 8D 01 00 00 00 90 C3");
}

#[test]
fn everything_gets_saved_and_put_back_in_the_same_order() {
    assert_eq!(hex(&asm32("pushall")), "60 9C");
    assert_eq!(hex(&asm32("popall")), "9D 61");
    let up = asm64("pushall");
    let down = asm64("popall");
    assert_eq!(up.len(), down.len());
    assert_eq!(&up[..4], &[0x9C, 0x50, 0x51, 0x52]);
    assert_eq!(&down[down.len() - 4..], &[0x5A, 0x59, 0x58, 0x9D]);
}

#[test]
fn the_ways_a_table_gets_written_wrong() {
    // the operands were commented out and the mnemonic left standing
    assert_eq!(hex(&asm32("mov\nnop")), "90");
    // a stray keystroke on the end of a jump
    assert_eq!(hex(&asm32("here:\njmp1 here")), "E9 FB FF FF FF");
    // a comma where the space belongs, and a missing one between operands
    assert_eq!(
        hex(&asm32(
            "nop
align,4"
        )),
        "90 90 90 90"
    );
    assert_eq!(hex(&asm32("mov al 01")), "B0 01");
    assert_eq!(hex(&asm32("cmp dword ptr,[esi+08],0")), "83 7E 08 00");
    // a bracket that was never opened, and one too many closing it
    assert_eq!(hex(&asm32("cmp edi+08],2")), "83 7F 08 02");
    assert_eq!(hex(&asm32("mov [eax+18]],1")), "C7 40 18 01 00 00 00");
    // a lone number on a line, which is never data anybody meant to write
    assert_eq!(hex(&asm32("here:4\nnop")), "90");
}

#[test]
fn arithmetic_the_author_left_for_the_assembler() {
    assert_eq!(hex(&asm32("mov eax,100-6")), "B8 FA 00 00 00");
    assert_eq!(hex(&asm32("mov eax,(dword)C")), "B8 0C 00 00 00");
    assert_eq!(hex(&asm32("mov eax,(int)0.5")), "B8 00 00 00 00");
    // no cast in front of it, so four bytes of float
    assert_eq!(hex(&asm32("mov eax,+Inf")), "B8 00 00 80 7F");
    assert_eq!(hex(&asm32("mov eax,1.0")), "B8 00 00 80 3F");
    // a character then the byte above it
    assert_eq!(hex(&asm32("mov ax,'_'0")), "66 B8 5F 00");
}

#[test]
fn a_pointer_chain_is_left_for_the_process_to_work_out() {
    let mut known = HashMap::new();
    known.insert("[[base+10]+20]".to_string(), 0x4000u64);
    let out = assemble("mov eax,[[[base+10]+20]]", 0x1000, Bits::X86, &known).unwrap();
    assert_eq!(hex(&out.bytes), "8B 05 00 40 00 00");
}

#[test]
fn two_names_added_together_resolve_to_the_sum() {
    let mut known = HashMap::new();
    known.insert("adreslist".to_string(), 0x4000u64);
    known.insert("changed".to_string(), 0x30u64);
    let out = assemble("mov [adreslist+changed],0", 0x1000, Bits::X86, &known).unwrap();
    assert_eq!(hex(&out.bytes), "C7 05 30 40 00 00 00 00 00 00");
}
