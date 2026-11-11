//! Words and excerpts against what SQLite FTS5 gave for the same input
//! (`unicode61` with its defaults, and `snippet(…, '**', '**', '…', 12)`),
//! recorded with SQLite 3.45.1. kkt-db-sqlite's tests compare against
//! FTS5 itself, over many more inputs.

use super::*;

fn texts(text: &str) -> Vec<String> {
    words(text).into_iter().map(|w| w.text).collect()
}

#[test]
fn words_are_split_and_folded_as_fts5_did() {
    let cases: &[(&str, &[&str])] = &[
        ("Café naïve résumé", &["cafe", "naive", "resume"]),
        ("e\u{301}cole", &["ecole"]),
        ("हिन्दी", &["ह", "न", "द"]),
        ("ภาษาไทย", &["ภาษาไทย"]),
        ("😀rocket🚀x", &["rocket", "x"]),
        ("\u{e000}pua", &["\u{e000}pua"]),
        ("٣٤ ½ Ⅻ", &["٣٤", "½", "ⅻ"]),
        (
            "Straße İstanbul ΣΊΣΥΦΟΣ ǅ",
            &["straße", "istanbul", "σίσυφοσ", "ǆ"],
        ),
        ("Ǆemal łódź Æsir œuvre", &["ǆemal", "łodz", "æsir", "œuvre"]),
        (
            "PDF.js long-poll a_b a'b",
            &["pdf", "js", "long", "poll", "a", "b", "a", "b"],
        ),
        ("中文字符", &["中文字符"]),
        ("x\u{200b}y", &["x", "y"]),
        ("ﬁ ﬀ Ａｂｃ ø đ ħ", &["ﬁ", "ﬀ", "ａｂｃ", "ø", "đ", "ħ"]),
        ("-- 🚀 ...", &[]),
    ];
    for (text, expected) in cases {
        assert_eq!(texts(text), *expected, "words of {text:?}");
    }
}

#[test]
fn a_word_knows_where_it_stands() {
    let text = "  Héllo, wörld!";
    let found = words(text);
    assert_eq!(found.len(), 2);
    assert_eq!(&text[found[0].start..found[0].end], "Héllo");
    assert_eq!(&text[found[1].start..found[1].end], "wörld");
}

#[test]
fn terms_drop_those_with_no_words() {
    assert_eq!(
        terms(&["PDF.js", "--", "🚀", "long poll"]),
        vec![vec!["pdf", "js"], vec!["long", "poll"]]
    );
    assert!(terms::<&str>(&[]).is_empty());
}

#[test]
fn a_text_matches_when_every_term_appears_in_order() {
    let text = "a long-poll loop, then the PDF.js viewer";
    let m = |raw: &[&str]| matches(text, &terms(raw));
    assert!(m(&["long poll"]));
    assert!(m(&["LONG-POLL", "pdf js"]));
    assert!(m(&["viewer", "loop"]));
    assert!(!m(&["poll long"]));
    assert!(!m(&["longpoll"]));
    assert!(!m(&["viewer", "missing"]));
    assert!(m(&["--", "viewer"]), "a term with no words is dropped");
    assert!(!m(&["--"]), "no terms match nothing");
}

#[test]
fn excerpts_match_fts5_snippets() {
    let cases: &[(&str, &[&str], &str)] = &[
        (
            "The parser failed on line three.",
            &["parser"],
            "The **parser** failed on line three.",
        ),
        (
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen fifteen parser sixteen seventeen eighteen nineteen twenty",
            &["parser"],
            "…ten eleven twelve thirteen fourteen fifteen **parser** sixteen seventeen eighteen nineteen twenty",
        ),
        (
            "parser one two three four five six seven eight nine ten eleven twelve thirteen fourteen",
            &["parser"],
            "**parser** one two three four five six seven eight nine ten eleven…",
        ),
        (
            "one two three four five six seven eight nine ten eleven twelve thirteen fourteen parser",
            &["parser"],
            "…four five six seven eight nine ten eleven twelve thirteen fourteen **parser**",
        ),
        (
            "We use PDF.js for rendering; pdf js is fine.",
            &["PDF.js"],
            "We use **PDF.js** for rendering; **pdf js** is fine.",
        ),
        (
            "a long-poll loop, then long poll again, not longpoll",
            &["long poll"],
            "a **long-poll** loop, then **long poll** again, not longpoll",
        ),
        (
            "alpha beta gamma delta. Epsilon zeta eta theta iota kappa lambda mu nu xi omicron pi rho sigma tau upsilon alpha",
            &["alpha", "sigma"],
            "…kappa lambda mu nu xi omicron pi rho **sigma** tau upsilon **alpha**",
        ),
        (
            "First sentence here. Second sentence talks about kkt search: and the parser too, with more words after it to fill",
            &["parser"],
            "…and the **parser** too, with more words after it to fill",
        ),
        (
            "Café naïve résumé — Cafe NAIVE",
            &["cafe", "naive"],
            "**Café** **naïve** résumé — **Cafe** **NAIVE**",
        ),
        (
            "x y z x y z x y z x y z x y z x y z",
            &["y z", "x"],
            "**x** **y z** **x** **y z** **x** **y z** **x** **y z**…",
        ),
        (
            "  leading spaces then the word match and trailing punctuation!!!",
            &["match"],
            "  leading spaces then the word **match** and trailing punctuation!!!",
        ),
        ("match", &["match"], "**match**"),
        ("x a b c y", &["a b", "b c"], "x **a b c** y"),
        (
            "x f1 f2 f3 f4 f5 f6 f7 f8 f9 f10 long poll f13",
            &["long poll", "x"],
            "**x** f1 f2 f3 f4 f5 f6 f7 f8 f9 f10 **long**…",
        ),
        ("aa aa aa end", &["aa aa"], "**aa aa aa** end"),
        ("one a b c d two", &["a b c", "b"], "one **a b c** d two"),
        (
            "a b c d e f g h i j k l m n o p q r s t u v w x y z match a b",
            &["match", "b"],
            "…r s t u v w x y z **match** a **b**",
        ),
        (
            "w1 w2 w3 w4 w5 w6 w7 w8 w9 w10 w11 w12 w13 w14 w15: w16 w17 target w18 w19 w20 w21 w22 w23 w24 w25",
            &["target"],
            "…w16 w17 **target** w18 w19 w20 w21 w22 w23 w24 w25",
        ),
        (
            "hello -- world 🚀 rocket",
            &["--", "hello"],
            "**hello** -- world 🚀 rocket",
        ),
        (
            "aa bb aa bb aa bb aa bb aa bb aa bb aa bb aa bb aa bb cc",
            &["aa bb", "cc"],
            "…bb **aa bb** **aa bb** **aa bb** **aa bb** **aa bb** **cc**",
        ),
    ];
    for (text, raw, expected) in cases {
        assert_eq!(
            excerpt(text, &terms(raw), "**", "**", "…", 12),
            *expected,
            "excerpt of {text:?} for {raw:?}"
        );
    }
}
