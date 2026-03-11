//! Sanity tests for sisvsim.
//! These verify basic functionality of the lexer, parser, and AST generation.

use sisvsim::*;
use sisvsim::lexer::token::TokenKind;
use sisvsim::ast::*;
use sisvsim::ast::module::*;
use sisvsim::ast::decl::*;

// ═══════════════════════════════════════════════════════════════════
// LEXER SANITY TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_lex_empty() {
    let tokens = tokenize("");
    assert_eq!(tokens.len(), 1);
    assert_eq!(tokens[0].kind, TokenKind::Eof);
}

#[test]
fn test_lex_keywords() {
    let tokens = tokenize("module endmodule wire logic always begin end");
    let kinds: Vec<_> = tokens.iter().map(|t| t.kind).collect();
    assert_eq!(kinds[0], TokenKind::KwModule);
    assert_eq!(kinds[1], TokenKind::KwEndmodule);
    assert_eq!(kinds[2], TokenKind::KwWire);
    assert_eq!(kinds[3], TokenKind::KwLogic);
    assert_eq!(kinds[4], TokenKind::KwAlways);
    assert_eq!(kinds[5], TokenKind::KwBegin);
    assert_eq!(kinds[6], TokenKind::KwEnd);
}

#[test]
fn test_lex_identifiers() {
    let tokens = tokenize("foo bar_baz my$id _start");
    assert_eq!(tokens[0].kind, TokenKind::Identifier);
    assert_eq!(tokens[0].text, "foo");
    assert_eq!(tokens[1].text, "bar_baz");
    assert_eq!(tokens[2].text, "my$id");
    assert_eq!(tokens[3].text, "_start");
}

#[test]
fn test_lex_escaped_identifier() {
    let tokens = tokenize("\\my-signal ");
    assert_eq!(tokens[0].kind, TokenKind::EscapedIdentifier);
    assert_eq!(tokens[0].text, "\\my-signal");
}

#[test]
fn test_lex_system_identifier() {
    let tokens = tokenize("$display $finish $time");
    assert_eq!(tokens[0].kind, TokenKind::SystemIdentifier);
    assert_eq!(tokens[0].text, "$display");
    assert_eq!(tokens[1].text, "$finish");
    assert_eq!(tokens[2].text, "$time");
}

#[test]
fn test_lex_integer_literals() {
    let tokens = tokenize("42 8'hFF 16'b1010_0101 32'o77 'sb1");
    assert_eq!(tokens[0].kind, TokenKind::IntegerLiteral);
    assert_eq!(tokens[0].text, "42");
    assert_eq!(tokens[1].kind, TokenKind::IntegerLiteral);
    assert_eq!(tokens[1].text, "8'hFF");
    assert_eq!(tokens[2].kind, TokenKind::IntegerLiteral);
    assert_eq!(tokens[2].text, "16'b1010_0101");
    assert_eq!(tokens[3].kind, TokenKind::IntegerLiteral);
    assert_eq!(tokens[3].text, "32'o77");
}

#[test]
fn test_lex_real_literal() {
    let tokens = tokenize("3.14 1.0e10 2.5E-3");
    assert_eq!(tokens[0].kind, TokenKind::RealLiteral);
    assert_eq!(tokens[1].kind, TokenKind::RealLiteral);
    assert_eq!(tokens[2].kind, TokenKind::RealLiteral);
}

#[test]
fn test_lex_string_literal() {
    let tokens = tokenize(r#""hello world" "with \"escape""#);
    assert_eq!(tokens[0].kind, TokenKind::StringLiteral);
    assert_eq!(tokens[1].kind, TokenKind::StringLiteral);
}

#[test]
fn test_lex_operators() {
    let tokens = tokenize("+ - * / ** == != === !== <= >= << >> <<< >>>");
    let expected = [
        TokenKind::Plus, TokenKind::Minus, TokenKind::Star, TokenKind::Slash,
        TokenKind::DoubleStar, TokenKind::Eq, TokenKind::Neq,
        TokenKind::CaseEq, TokenKind::CaseNeq,
        TokenKind::Leq, TokenKind::Geq,
        TokenKind::ShiftLeft, TokenKind::ShiftRight,
        TokenKind::ArithShiftLeft, TokenKind::ArithShiftRight,
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(tokens[i].kind, *exp, "token {} mismatch", i);
    }
}

#[test]
fn test_lex_assignment_operators() {
    let tokens = tokenize("+= -= *= /= %= &= |= ^=");
    let expected = [
        TokenKind::PlusAssign, TokenKind::MinusAssign,
        TokenKind::StarAssign, TokenKind::SlashAssign,
        TokenKind::PercentAssign, TokenKind::AndAssign,
        TokenKind::OrAssign, TokenKind::XorAssign,
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(tokens[i].kind, *exp, "token {} mismatch", i);
    }
}

#[test]
fn test_lex_delimiters() {
    let tokens = tokenize("( ) [ ] { } ; : , . # @");
    let expected = [
        TokenKind::LParen, TokenKind::RParen,
        TokenKind::LBracket, TokenKind::RBracket,
        TokenKind::LBrace, TokenKind::RBrace,
        TokenKind::Semicolon, TokenKind::Colon,
        TokenKind::Comma, TokenKind::Dot,
        TokenKind::Hash, TokenKind::At,
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(tokens[i].kind, *exp, "token {} mismatch", i);
    }
}

#[test]
fn test_lex_comments() {
    let tokens = tokenize("a // line comment\nb /* block */ c");
    let idents: Vec<_> = tokens.iter()
        .filter(|t| t.kind == TokenKind::Identifier)
        .map(|t| t.text.as_str())
        .collect();
    assert_eq!(idents, vec!["a", "b", "c"]);
}

#[test]
fn test_lex_directives() {
    let tokens = tokenize("`define FOO `ifdef BAR");
    assert_eq!(tokens[0].kind, TokenKind::Directive);
    assert_eq!(tokens[0].text, "`define");
    assert_eq!(tokens[1].kind, TokenKind::Identifier);
    assert_eq!(tokens[2].kind, TokenKind::Directive);
    assert_eq!(tokens[2].text, "`ifdef");
}

#[test]
fn test_lex_special_operators() {
    let tokens = tokenize("++ -- -> ->> => <-> ## :: +: -:");
    let expected = [
        TokenKind::Increment, TokenKind::Decrement,
        TokenKind::Arrow, TokenKind::DoubleArrow,
        TokenKind::FatArrow, TokenKind::LogEquiv,
        TokenKind::HashHash, TokenKind::DoubleColon,
        TokenKind::PlusColon, TokenKind::MinusColon,
    ];
    for (i, exp) in expected.iter().enumerate() {
        assert_eq!(tokens[i].kind, *exp, "token {} mismatch", i);
    }
}

#[test]
fn test_lex_unbased_unsized() {
    let tokens = tokenize("'0 '1 'x 'z");
    for tok in &tokens[..4] {
        assert_eq!(tok.kind, TokenKind::UnbasedUnsizedLiteral);
    }
}

// ═══════════════════════════════════════════════════════════════════
// PARSER SANITY TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_parse_empty_module() {
    let result = parse_str("module top; endmodule").unwrap();
    assert_eq!(result.source_text.descriptions.len(), 1);
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert_eq!(m.name.name, "top");
        assert!(m.items.is_empty());
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_module_with_ports() {
    let result = parse_str("module foo(input logic a, output logic b); endmodule").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert_eq!(m.name.name, "foo");
        if let PortList::Ansi(ports) = &m.ports {
            assert_eq!(ports.len(), 2);
            assert_eq!(ports[0].name.name, "a");
            assert_eq!(ports[1].name.name, "b");
        } else { panic!("Expected ANSI ports"); }
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_module_with_parameters() {
    let result = parse_str("module foo #(parameter int WIDTH = 8)(input logic [WIDTH-1:0] data); endmodule").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert_eq!(m.params.len(), 1);
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_wire_declaration() {
    let result = parse_str("module m; wire [7:0] data; endmodule").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert!(!m.items.is_empty());
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_always_block() {
    let src = "module m;
        always_ff @(posedge clk) begin
            q <= d;
        end
    endmodule";
    let result = parse_str(src).unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        let found = m.items.iter().any(|item| matches!(item, ModuleItem::AlwaysConstruct(a) if a.kind == AlwaysKind::AlwaysFf));
        assert!(found, "Expected always_ff construct");
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_continuous_assign() {
    let result = parse_str("module m; assign y = a & b; endmodule").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        let found = m.items.iter().any(|item| matches!(item, ModuleItem::ContinuousAssign(_)));
        assert!(found, "Expected continuous assign");
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_module_instantiation() {
    let result = parse_str("module top; sub_mod u1(.a(x), .b(y)); endmodule").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        let found = m.items.iter().any(|item| matches!(item, ModuleItem::ModuleInstantiation(_)));
        assert!(found, "Expected module instantiation");
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_if_else() {
    let src = "module m; always_comb begin if (a) b = 1; else b = 0; end endmodule";
    let result = parse_str(src).unwrap();
    assert!(!result.has_errors(), "Parse should succeed without errors");
}

#[test]
fn test_parse_case_statement() {
    let src = "module m; always_comb begin case(sel) 2'b00: out = a; 2'b01: out = b; default: out = 0; endcase end endmodule";
    let result = parse_str(src).unwrap();
    assert!(!result.has_errors(), "Parse should succeed without errors");
}

#[test]
fn test_parse_for_loop() {
    let src = "module m; initial begin for (int i = 0; i < 10; i++) begin $display(i); end end endmodule";
    let result = parse_str(src).unwrap();
    assert_eq!(result.source_text.descriptions.len(), 1);
}

#[test]
fn test_parse_function() {
    let src = "module m;
        function int add(input int a, input int b);
            return a + b;
        endfunction
    endmodule";
    let result = parse_str(src).unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        let found = m.items.iter().any(|item| matches!(item, ModuleItem::FunctionDeclaration(_)));
        assert!(found, "Expected function declaration");
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_task() {
    let src = "module m;
        task my_task(input logic a, output logic b);
            b = ~a;
        endtask
    endmodule";
    let result = parse_str(src).unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        let found = m.items.iter().any(|item| matches!(item, ModuleItem::TaskDeclaration(_)));
        assert!(found);
    } else { panic!("Expected module"); }
}

#[test]
fn test_parse_package() {
    let src = "package my_pkg;
        typedef logic [7:0] byte_t;
        parameter int DEPTH = 16;
    endpackage";
    let result = parse_str(src).unwrap();
    if let Description::Package(p) = &result.source_text.descriptions[0] {
        assert_eq!(p.name.name, "my_pkg");
        assert_eq!(p.items.len(), 2);
    } else { panic!("Expected package"); }
}

#[test]
fn test_parse_interface() {
    let src = "interface my_if;
        logic valid;
        logic [7:0] data;
    endinterface";
    let result = parse_str(src).unwrap();
    if let Description::Interface(i) = &result.source_text.descriptions[0] {
        assert_eq!(i.name.name, "my_if");
    } else { panic!("Expected interface"); }
}

#[test]
fn test_parse_program() {
    let src = "program test;
        initial begin
            $display(\"Hello\");
            $finish;
        end
    endprogram";
    let result = parse_str(src).unwrap();
    if let Description::Program(p) = &result.source_text.descriptions[0] {
        assert_eq!(p.name.name, "test");
    } else { panic!("Expected program"); }
}

#[test]
fn test_parse_multiple_modules() {
    let src = "module a; endmodule module b; endmodule module c; endmodule";
    let result = parse_str(src).unwrap();
    assert_eq!(result.source_text.descriptions.len(), 3);
}

#[test]
fn test_parse_endlabel() {
    let result = parse_str("module top; endmodule : top").unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert_eq!(m.endlabel.as_ref().unwrap().name, "top");
    } else { panic!("Expected module"); }
}

// ═══════════════════════════════════════════════════════════════════
// PREPROCESSOR SANITY TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_preprocessor_define() {
    let src = "`define WIDTH 8\nmodule m; logic [`WIDTH-1:0] data; endmodule";
    let result = parse_str(src).unwrap();
    assert_eq!(result.source_text.descriptions.len(), 1);
}

#[test]
fn test_preprocessor_ifdef() {
    let src = "`define FOO\n`ifdef FOO\nmodule a; endmodule\n`else\nmodule b; endmodule\n`endif";
    let result = parse_str(src).unwrap();
    if let Description::Module(m) = &result.source_text.descriptions[0] {
        assert_eq!(m.name.name, "a");
    } else { panic!("Expected module a"); }
}

#[test]
fn test_preprocessor_ifndef() {
    let src = "`ifndef MISSING\nmodule yes; endmodule\n`endif";
    let result = parse_str(src).unwrap();
    assert_eq!(result.source_text.descriptions.len(), 1);
}

// ═══════════════════════════════════════════════════════════════════
// DIAGNOSTICS SANITY TESTS
// ═══════════════════════════════════════════════════════════════════

#[test]
fn test_error_recovery_missing_semi() {
    // Missing semicolon after endmodule shouldn't crash
    let result = parse_str("module top endmodule");
    assert!(result.is_ok());
    let r = result.unwrap();
    assert!(r.has_errors()); // Should report error about missing ;
}

#[test]
fn test_error_unexpected_token() {
    let result = parse_str("@@@ invalid tokens").unwrap();
    assert!(result.has_errors());
}
