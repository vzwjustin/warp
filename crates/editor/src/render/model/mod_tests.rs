use std::cell::Cell;
use std::sync::Arc;

use markdown_parser::{FormattedTextStyles, Hyperlink};
use rangemap::RangeSet;
use string_offset::CharOffset;
use sum_tree::SumTree;
use vec1::{Vec1, vec1};
use warpui::assets::asset_cache::AssetSource;
use warpui::color::ColorU;
use warpui::elements::ListIndentLevel;
use warpui::fonts::FamilyId;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::vec2f;
use warpui::text_layout::TextFrame;
use warpui::units::{IntoPixels, Pixels};

use super::debug::Describe;
use super::test_utils::{layout_paragraph, layout_paragraphs};
use super::{
    BlockItem, BlockLocation, COMMAND_SPACING, CellLayout, DEFAULT_BLOCK_SPACINGS,
    HiddenBlockConfig, ImageBlockConfig, LaidOutTable, ParagraphBlock, RenderState,
    TableBlockConfig, TableStyle, table_offset_map,
};
use crate::content::edit::ParsedUrl;
use crate::content::text::{
    BufferBlockStyle, CodeBlockType, FormattedTable, FormattedTextFragment, table_cell_offset_maps,
};
use crate::render::model::test_utils::{TEST_STYLES, laid_out_paragraph, mock_paragraph};
use crate::render::model::{
    Height, LayoutSummary, LineCount, RenderedSelection, SoftWrapPoint, TEXT_SPACING,
};

#[test]
fn test_height() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let mut content = SumTree::new();
    // Height: 24
    content.push(mock_paragraph(24., 1., 1));
    // Height: 48
    content.push(mock_paragraph(48., 1., 2));
    // Height: 24
    content.push(mock_paragraph(24., 1., 3));
    // Height: 24
    content.push(mock_paragraph(24., 1., 4));
    // Height: 32
    content.push(mock_paragraph(32., 1., 5));
    render_state.set_content(content);

    // This includes all content plus the trailing newline marker.
    assert_eq!(render_state.height(), 176.0.into_pixels());
    let content = render_state.content.borrow();
    let mut cursor = content.cursor::<Height, Height>();
    // Ensure we can seek in between items for scrolling.
    cursor.seek(&Height::from(64.), sum_tree::SeekBias::Left);
    assert_eq!(
        cursor.item().expect("Seek succeeded").height().as_f32(),
        48.
    );
    assert_eq!(cursor.start().into_pixels().as_f32(), 24.);
    assert_eq!(cursor.end().into_pixels().as_f32(), 72.);

    let end = cursor.slice(&Height::from(152.), sum_tree::SeekBias::Right);
    assert_eq!(
        end.summary(),
        LayoutSummary {
            content_length: 14.into(),
            height: 48. + 24. + 24. + 32.,
            width: (17.).into_pixels(),
            lines: LineCount(4),
            item_count: 4,
        }
    );
}

#[test]
fn test_is_entire_range_of_type_matches_exact_block_ranges() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        200.0.into_pixels(),
        160.0.into_pixels(),
    );
    let mut content = SumTree::new();
    content.push(laid_out_paragraph("Before\n", &TEST_STYLES, 200.0));
    let mermaid_start = content.extent::<CharOffset>();
    content.push(BlockItem::MermaidDiagram {
        content_length: 14.into(),
        asset_source: AssetSource::Bundled {
            path: "bundled/svg/test.svg",
        },
        config: ImageBlockConfig {
            width: 120.0.into_pixels(),
            height: 40.0.into_pixels(),
            spacing: COMMAND_SPACING,
        },
    });
    let mermaid_end = content.extent::<CharOffset>();
    content.push(laid_out_paragraph("After\n", &TEST_STYLES, 200.0));
    model.set_content(content);

    assert!(
        model.is_entire_range_of_type(&(mermaid_start..mermaid_end), |item| matches!(
            item,
            BlockItem::MermaidDiagram { .. }
        ),)
    );
    assert!(!model.is_entire_range_of_type(
        &(mermaid_start + CharOffset::from(1)..mermaid_end),
        |item| matches!(item, BlockItem::MermaidDiagram { .. }),
    ));
    assert!(!model.is_entire_range_of_type(
        &(mermaid_start..mermaid_end - CharOffset::from(1)),
        |item| matches!(item, BlockItem::MermaidDiagram { .. }),
    ));
    assert!(
        !model.is_entire_range_of_type(&(CharOffset::zero()..mermaid_end), |item| matches!(
            item,
            BlockItem::MermaidDiagram { .. }
        ),)
    );
}

#[test]
fn test_width() {
    let mut render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let mut content = SumTree::new();
    // Width 25.
    content.push(mock_paragraph(24., 10., 1));
    // Width: 10.
    content.push(mock_paragraph(48., 25., 2));
    render_state.set_content(content);

    // This includes all content plus the trailing newline marker.
    assert_eq!(render_state.width(), (41.).into_pixels());
    let content = render_state.content.borrow();
    let mut cursor = content.cursor::<Height, Height>();
    let end = cursor.slice(&Height::from(40.), sum_tree::SeekBias::Right);
    assert_eq!(
        end.summary(),
        LayoutSummary {
            content_length: 1.into(),
            height: 24.,
            width: (26.).into_pixels(),
            lines: LineCount(1),
            item_count: 1,
        }
    );
}

#[test]
fn test_soft_wrap_point() {
    /// Helper to convert a character count to a pixel x-offset, accounting for plain-text spacing.
    fn char_x(chars: usize) -> Pixels {
        TEXT_SPACING.left_offset() + (chars as f32 * TEST_STYLES.base_text.font_size).into_pixels()
    }

    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    let mut content = SumTree::new();
    // This paragraph soft-wraps to 2 lines and includes chars 0-7.
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // This paragraph fits on a single line and includes chars 8-12.
    content.push(laid_out_paragraph("ABCD\n", &TEST_STYLES, 40.));
    // This paragraph soft-wraps to 2 lines and includes chars 13-20.
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // This line is empty and includes char 21.
    content.push(laid_out_paragraph("\n", &TEST_STYLES, 40.));
    // This paragraph fits on a single line and includes chars 22-25.
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, 40.));
    assert_eq!(content.extent::<CharOffset>(), CharOffset::from(26));
    assert_eq!(content.extent::<LineCount>().as_usize(), 7);
    model.set_content(content);

    // Last point on the first softwrapped line.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(3)),
        SoftWrapPoint::new(0, char_x(3))
    );

    // A point slightly closer to 2 than 3 should round to 2.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(2) + 4.0.into_pixels())),
        CharOffset::from(2)
    );

    // A point slightly closer to 3 than 2 should round to 3.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(3) - 4.0.into_pixels())),
        CharOffset::from(3)
    );

    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(0, char_x(4))),
        CharOffset::from(4)
    );

    // Point on the second softwrapped line in the first paragraph.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(7)),
        SoftWrapPoint::new(1, char_x(3))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(1, char_x(3))),
        CharOffset::from(7)
    );

    // Non-softwrapped line should work as well.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(10)),
        SoftWrapPoint::new(2, char_x(2))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(2, char_x(2))),
        CharOffset::from(10)
    );

    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(19)),
        SoftWrapPoint::new(4, char_x(2))
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(4, char_x(2))),
        CharOffset::from(19)
    );

    // Softwrapping on an empty line should work.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(21)),
        SoftWrapPoint::new(5, TEXT_SPACING.left_offset())
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, Pixels::zero())),
        CharOffset::from(21)
    );

    // Out of bound points should be bounded to the trailing newline.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(40)),
        SoftWrapPoint::new(8, Pixels::zero())
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(7, Pixels::zero())),
        CharOffset::from(26)
    );

    // Points are bounded to their line's contents.
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, char_x(3))),
        CharOffset::from(21)
    );
    assert_eq!(
        model.softwrap_point_to_offset(SoftWrapPoint::new(5, char_x(2))),
        CharOffset::from(21)
    );
}

// ── CLD-558 characterization tests ───────────────────────────────────────────
//
// These tests pin the CURRENT behaviour of `offset_to_softwrap_point` and
// `softwrap_point_to_offset`.  They document the coordinate-system invariant
// that the render model's SumTree uses 0-indexed CharOffsets, where
// `content_length` is the cumulative size of blocks starting from 0.  They
// must NOT be changed to assert "desired" future behaviour – their value comes
// precisely from asserting what the code actually does today, so regressions
// are caught immediately.
//
// Key invariant:
//   render offset  =  buffer (1-indexed) offset  −  1
//
// For paragraph blocks:
//   • `offset_to_softwrap_point(R)` maps render offset R → (row, pixel_column).
//   • `softwrap_point_to_offset((row, col))` maps back to a render offset.
//   • Round-tripping via a SoftWrapPoint is lossless for caret positions on
//     the same character (though rounding to the nearest glyph can occur for
//     sub-character pixel values).
//   • `max_offset()` = total SumTree content_length − 1, and
//     `softwrap_point_to_offset` for an out-of-bounds row returns `max_offset()`.
//
// For the `navigate_line*` callers (in selection.rs) the ±1 adjustments are
// because those functions receive *buffer* 1-indexed offsets, convert to
// render coordinates (−1) before calling `offset_to_softwrap_point`, then
// convert back (+1) after calling `softwrap_point_to_offset`.
//
// Exception – end-of-line forwards path (`navigate_line_boundary` Forwards):
//   `softwrap_point_to_offset(start_of_next_row)` returns the render offset
//   of the first character of the next soft-wrap row. By the layout
//   construction invariant, this value equals the *buffer* offset of the
//   trailing newline of the current hard-wrapped line, so no additional ±1
//   is required in that specific code path.

/// Builds a model and tree identical to `test_soft_wrap_point`.
/// Returns the model and also a `char_x` helper as a closure.
fn make_soft_wrap_model() -> (RenderState, impl Fn(usize) -> Pixels) {
    let model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    let char_x = |chars: usize| -> Pixels {
        TEXT_SPACING.left_offset() + (chars as f32 * TEST_STYLES.base_text.font_size).into_pixels()
    };
    (model, char_x)
}

/// Round-trip characterization: offset → softwrap_point → offset
///
/// Verifies that for every tested render offset the round-trip
/// `softwrap_point_to_offset(offset_to_softwrap_point(r)) == r`
/// holds for the current implementation.
#[test]
fn test_cld558_round_trip_offset_to_swp_to_offset() {
    let (mut model, _char_x) = make_soft_wrap_model();
    let mut content = SumTree::new();
    // "ABCDEFG\n"  → render offsets 0..8  (content_length = 8)
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // "ABCD\n"     → render offsets 8..13 (content_length = 5)
    content.push(laid_out_paragraph("ABCD\n", &TEST_STYLES, 40.));
    // "ABCDEFG\n"  → render offsets 13..21
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    // "\n"         → render offsets 21..22
    content.push(laid_out_paragraph("\n", &TEST_STYLES, 40.));
    // "ABC\n"      → render offsets 22..26
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, 40.));
    model.set_content(content);

    // Probe a representative set of render offsets and verify round-trip.
    // Note: `offset_to_softwrap_point` snaps to the nearest glyph boundary on
    // the pixel axis, so the inverse `softwrap_point_to_offset` may not recover
    // the exact input for every pixel-level column – but for offsets that
    // correspond to actual caret positions (integer render offsets) the
    // round-trip is exact.
    for render_offset in [0usize, 1, 3, 7, 8, 9, 12, 13, 14, 20, 21, 22, 25, 26] {
        let r = CharOffset::from(render_offset);
        let swp = model.offset_to_softwrap_point(r);
        let recovered = model.softwrap_point_to_offset(swp);
        assert_eq!(
            recovered, r,
            "round-trip failed for render offset {render_offset}: \
             offset_to_softwrap_point = {swp:?}, but softwrap_point_to_offset = {recovered:?}"
        );
    }
}

/// Round-trip characterization: softwrap_point → offset → softwrap_point
///
/// Verifies that for well-formed SoftWrapPoints the inverse round-trip
/// `offset_to_softwrap_point(softwrap_point_to_offset(p))` recovers a
/// point on the same row (column may differ due to nearest-glyph snapping).
#[test]
fn test_cld558_round_trip_swp_to_offset_to_swp() {
    let (mut model, char_x) = make_soft_wrap_model();
    let mut content = SumTree::new();
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    content.push(laid_out_paragraph("ABCD\n", &TEST_STYLES, 40.));
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.));
    content.push(laid_out_paragraph("\n", &TEST_STYLES, 40.));
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, 40.));
    model.set_content(content);

    // For each (row, col) pair: the recovered offset must lie within the block
    // for that row, and the re-computed softwrap point must be on the same row.
    let cases: &[(u32, usize)] = &[
        (0, 0),
        (0, 3), // first soft-wrap line of para 1
        (1, 0),
        (1, 3), // second soft-wrap line of para 1
        (2, 0),
        (2, 2), // para 2 (single line)
        (3, 0),
        (3, 2), // first soft-wrap line of para 3
        (4, 0),
        (4, 2), // second soft-wrap line of para 3
        (5, 0), // empty line (para 4 = "\n")
        (6, 0),
        (6, 2), // para 5 ("ABC\n")
    ];
    for &(row, chars) in cases {
        let col = char_x(chars);
        let swp = SoftWrapPoint::new(row, col);
        let offset = model.softwrap_point_to_offset(swp);
        let recovered_swp = model.offset_to_softwrap_point(offset);
        assert_eq!(
            recovered_swp.row(),
            row,
            "round-trip row mismatch for ({row}, {chars}): \
             softwrap_point_to_offset = {offset:?}, recovered row = {}",
            recovered_swp.row()
        );
    }
}

/// Characterization: max_offset equals total SumTree content_length minus one.
///
/// This value is the render-model analogue of the buffer's `max_charoffset()`.
/// The −1 exists because the SumTree always ends with a TrailingNewLine
/// placeholder that has content_length = 1; `max_offset` intentionally
/// excludes it so that out-of-bounds `softwrap_point_to_offset` calls clamp
/// to a meaningful position.
#[test]
fn test_cld558_max_offset_is_content_length_minus_one() {
    let (mut model, _char_x) = make_soft_wrap_model();
    let mut content = SumTree::new();
    content.push(laid_out_paragraph("ABCDEFG\n", &TEST_STYLES, 40.)); // len 8
    content.push(laid_out_paragraph("ABCD\n", &TEST_STYLES, 40.)); // len 5
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, 40.)); // len 4
    model.set_content(content);
    // Total pushed content = 8+5+4 = 17.  set_content appends a TrailingNewLine
    // (len 1), making the SumTree extent 18.  max_offset = 18 − 1 = 17.
    assert_eq!(model.max_offset(), CharOffset::from(17));
    // Out-of-bounds row clamps to max_offset().
    let oob_row = SoftWrapPoint::new(999, Pixels::zero());
    assert_eq!(
        model.softwrap_point_to_offset(oob_row),
        CharOffset::from(17)
    );
}

/// Characterization: the ±1 shift between buffer and render coordinates.
///
/// Demonstrates the invariant directly: for a simple one-paragraph render
/// model, `render_offset = buffer_offset − 1`.
///
/// In this test we build the render tree directly (without going through the
/// buffer), so render offsets start at 0.  The TrailingNewLine added by
/// `set_content` has `start_char_offset = content_length_of_paragraph`, which
/// equals `buffer_offset_of_trailing_newline` (1-indexed).  Therefore
/// `softwrap_point_to_offset(start_of_next_row) == buffer_newline_offset`
/// without any explicit ±1 — this is the property that `navigate_line_boundary`
/// Forwards exploits.
#[test]
fn test_cld558_render_vs_buffer_offset_invariant() {
    let (mut model, char_x) = make_soft_wrap_model();
    let mut content = SumTree::new();
    // "ABC\n"  →  render offsets 0..4  (A=0, B=1, C=2, \n slot=3)
    content.push(laid_out_paragraph("ABC\n", &TEST_STYLES, f32::MAX));
    model.set_content(content);

    // render offset 0 → 'A' (first character).
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(0)),
        SoftWrapPoint::new(0, char_x(0)),
    );
    // render offset 2 → 'C'.
    assert_eq!(
        model.offset_to_softwrap_point(CharOffset::from(2)),
        SoftWrapPoint::new(0, char_x(2)),
    );
    // render offset 3 is the exclusive end of the paragraph block (the \n slot).
    // `softwrap_point_to_offset` for the start of the next row returns 3.
    // This equals the *buffer* 1-indexed offset of the '\n' in a one-paragraph
    // buffer (buffer has block marker at 0, 'A'=1, 'B'=2, 'C'=3, '\n'=4 →
    // actually no: "ABC\n" has 4 chars, buffer range is 1..5; '\n'=4).
    // Wait — in the render tree, the paragraph's content_length = 4,
    // the TrailingNewLine starts at render offset 4.
    // In the buffer, '\n' is at buffer offset 4 (1-indexed: A=1,B=2,C=3,\n=4).
    // render_end_of_paragraph (4)  ==  buffer_offset_of_newline (4).  ✓
    let trailing_nl_start = model.softwrap_point_to_offset(SoftWrapPoint::new(1, Pixels::zero()));
    assert_eq!(trailing_nl_start, CharOffset::from(4));
}

#[test]
fn test_character_bounds() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    let mut content = SumTree::new();
    // This paragraph soft-wraps to 2 lines and includes chars 0-7.
    content.push(laid_out_paragraph(
        "ABCDEFG\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    // This paragraph soft-wraps to 2 lines and includes chars 8-14.
    content.push(laid_out_paragraph(
        "HIJKLMN\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    model.set_content(content);

    // Due to the minimum block height, there is 2px of top spacing.

    let char_size = vec2f(10., 10.);

    // The middle of the first line.
    assert_eq!(
        model.character_bounds(2.into()),
        Some(RectF::new(vec2f(20., 2.), char_size))
    );

    // The first character of the second soft-wrapped line.
    assert_eq!(
        model.character_bounds(4.into()),
        Some(RectF::new(vec2f(0., 12.), char_size))
    );

    // The middle of the first line of the second paragraph.
    assert_eq!(
        model.character_bounds(9.into()),
        Some(RectF::new(vec2f(10., 26.), char_size))
    );

    // The end of the first line of the second paragraph.
    assert_eq!(
        model.character_bounds(11.into()),
        Some(RectF::new(vec2f(30., 26.), char_size))
    );

    // The middle of the second line of the second paragraph.
    assert_eq!(
        model.character_bounds(13.into()),
        Some(RectF::new(vec2f(10., 36.), char_size))
    );
}

#[test]
fn test_non_empty_content_can_hide_final_trailing_newline() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    model.set_show_final_trailing_newline_when_non_empty(false);

    let mut content = SumTree::new();
    content.push(BlockItem::RunnableCodeBlock {
        paragraph_block: ParagraphBlock::new(layout_paragraphs(
            "First\nSecond\n",
            &TEST_STYLES,
            &BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            },
            model.viewport.width().as_f32(),
        )),
        code_block_type: Default::default(),
        pending_mermaid_asset: None,
    });
    model.set_content(content);

    assert_eq!(model.blocks(), 1);
    assert_eq!(model.height(), 104.0.into_pixels());
}

#[test]
fn test_empty_content_keeps_final_trailing_newline_when_suppressed() {
    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    model.set_show_final_trailing_newline_when_non_empty(false);

    assert_eq!(model.blocks(), 1);
    assert_eq!(model.height(), 24.0.into_pixels());
}

#[test]
fn test_ordered_list_counting() {
    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 30.0.into_pixels());
    let mut content = SumTree::new();
    content.push(laid_out_paragraph(
        "Text\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "One\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "Two\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "Three\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(laid_out_paragraph(
        "Middle\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: Some(10),
        paragraph: layout_paragraph(
            "A\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "B\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(laid_out_paragraph(
        "Last\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::One,
        number: None,
        paragraph: layout_paragraph(
            "i\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Three,
        number: None,
        paragraph: layout_paragraph(
            "iii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Three,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::OrderedList {
        indent_level: ListIndentLevel::Two,
        number: None,
        paragraph: layout_paragraph(
            "ii\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    model.set_content(content);

    // Map blocks to start offsets for test readability
    let block_starts = [0, 5, 9, 13, 19, 26, 28, 30, 35, 37, 40, 44, 47].map(CharOffset::from);

    // At the start of the buffer, there's no ordered list, so the numbering starts at 1.
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If we scroll to just _above_ the first ordered list item, the numbering is still 1.
    model.scroll_near_block(block_starts[1], -2.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If the first ordered list item is partially out of viewport, that still counts - numbering
    // should start at 1.
    model.viewport.scroll((-6.).into_pixels(), model.height());
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // Scroll to the second ordered list item, the numbering should now start at 2.
    model.scroll_near_block(block_starts[2], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 2);

    // Likewise for the third ordered list item.
    model.scroll_near_block(block_starts[3], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 3);

    // Because the plain-text paragraph in the middle isn't an ordered list, we won't bother
    // calculating an initial numbering for it.
    model.scroll_near_block(block_starts[4], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 1);

    // If we scroll to the second list, after the paragraph, numbering resets to its start number.
    model.scroll_near_block(block_starts[5], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, Some(10)).label_index, 10);
    model.scroll_near_block(block_starts[6], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(0, None).label_index, 11);

    // Test numbering across indent levels, with the last list.
    model.scroll_near_block(block_starts[11], 1.);
    let mut numbering = model.viewport_list_numbering();
    assert_eq!(numbering.advance(1, None).label_index, 2);
}

#[test]
fn test_first_line_bounds() {
    // Create a model with:
    // * Plain text
    // * A list
    // * A code block
    // * A trailing newline
    // We then test that the first line of each is correct.

    let mut model = RenderState::new_for_test(
        TEST_STYLES.clone(),
        100.0.into_pixels(),
        200.0.into_pixels(),
    );
    let mut content = SumTree::new();
    // This paragraph is 4 soft-wrapped lines.
    content.push(laid_out_paragraph(
        "This is a soft-wrapped paragraph\n",
        &TEST_STYLES,
        model.viewport.width().as_f32(),
    ));
    content.push(BlockItem::UnorderedList {
        indent_level: ListIndentLevel::One,
        paragraph: layout_paragraph(
            "List\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::One,
            },
            model.viewport.width().as_f32(),
        ),
    });
    // This list item is 3 soft-wrapped lines.
    content.push(BlockItem::UnorderedList {
        indent_level: ListIndentLevel::Two,
        paragraph: layout_paragraph(
            "Nested and soft-wrapped\n",
            &TEST_STYLES,
            &BufferBlockStyle::OrderedList {
                number: None,
                indent_level: ListIndentLevel::Two,
            },
            model.viewport.width().as_f32(),
        ),
    });
    content.push(BlockItem::RunnableCodeBlock {
        paragraph_block: ParagraphBlock::new(layout_paragraphs(
            "First\nSecond\n",
            &TEST_STYLES,
            &BufferBlockStyle::CodeBlock {
                code_block_type: CodeBlockType::Shell,
            },
            model.viewport.width().as_f32(),
        )),
        code_block_type: Default::default(),
        pending_mermaid_asset: None,
    });
    model.set_content(content);

    let content = model.content();
    let text_block = content
        .block_at_offset(CharOffset::zero())
        .expect("Block should exist");
    // Because the paragraph is soft-wrapped, it doesn't need centering.
    assert_eq!(
        text_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(vec2f(0., 0.), vec2f(100., 10.))
    );
    assert_eq!(text_block.item.height().as_f32(), 40.);

    let list_block = content
        .block_at_offset(CharOffset::from(33))
        .expect("Block should exist");
    assert_eq!(
        list_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 44.),
            vec2f(
                64., /* 4px margin + 20px list padding + 40px of text */
                10.
            )
        )
    );
    assert_eq!(list_block.item.height().as_f32(), 18.);

    let list_block_2 = content
        .block_at_offset(CharOffset::from(38))
        .expect("Block should exist");
    assert_eq!(
        list_block_2
            .first_line_bounds()
            .expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 62. /* 58px y-offset + 4px margin */),
            vec2f(
                144., /* 4px margin + 40px list padding + 10px of text - the test layout logic doesn't account for spacing */
                10.
            )
        )
    );
    assert_eq!(list_block_2.item.height(), 38.0.into_pixels());

    let code_block = content
        .block_at_offset(CharOffset::from(62))
        .expect("Block should exist");
    assert_eq!(
        code_block.first_line_bounds().expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 104. /* 96px y-offset + 8px margin */),
            vec2f(
                70., /* 4px margin + 16px padding + 50px text */
                16.  /* 16px padding area */
            )
        )
    );
    assert_eq!(
        code_block.item.height(),
        104.0.into_pixels() /* 3 lines of text due to newlines + all the padding + footer*/
    );

    let trailing_block = content
        .block_at_offset(CharOffset::from(76))
        .expect("Block should exist");
    assert_eq!(
        trailing_block
            .first_line_bounds()
            .expect("Bounds should exist"),
        RectF::new(
            vec2f(0., 207. /* 200px y-offset + 7px centering */,),
            vec2f(1. /* 1px cursor */, 10.)
        )
    )
}

#[test]
fn test_scroll_snapshot() {
    // Lay out the content at the current viewport width.
    fn layout_content(model: &mut RenderState) {
        let mut content = SumTree::new();
        content.push(laid_out_paragraph(
            "AAAABBBBCCCC\n",
            &TEST_STYLES,
            model.viewport().width().as_f32(),
        ));
        content.push(laid_out_paragraph(
            "DDDDEEEEFFFFGGGG\n",
            &TEST_STYLES,
            model.viewport().width().as_f32(),
        ));
        model.set_content(content);
    }

    let mut model =
        RenderState::new_for_test(TEST_STYLES.clone(), 40.0.into_pixels(), 60.0.into_pixels());
    layout_content(&mut model);

    let content = model.content();
    // Verify the height of each block. Each text paragraph has 10px per soft-wrapped line with a
    // 24px minimum height. The trailing newline block is 24px high.
    assert_eq!(
        content
            .block_at_offset(CharOffset::zero())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        30.
    );
    assert_eq!(
        content
            .block_at_offset(13.into())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        40.
    );
    assert_eq!(
        content
            .block_at_offset(30.into())
            .expect("Block should exist")
            .item
            .height()
            .as_f32(),
        24.
    );
    drop(content);

    // Scroll so that the EEEE line is at the top of the viewport.
    model.viewport.scroll((-44.).into_pixels(), model.height());
    let scroll_position = model.snapshot_scroll_position();
    assert_eq!(scroll_position.first_character_offset(), 13.into());

    // Now, double the viewport width, halving the number of soft-wrapped lines.
    model
        .viewport
        .set_size(vec2f(80., 60.), model.width(), model.height());

    // At first, the content will not have been laid out again, so the scroll position is
    // unaffected.
    assert_eq!(model.viewport.scroll_top(), 34.0.into_pixels());
    // After laying out again, each block is exactly 24px high (the two soft-wrapped blocks are
    // below the minimum height otherwise).
    layout_content(&mut model);
    assert_eq!(model.height().as_f32(), 24. * 3.);

    // Restore the scroll position at the new height. It should still start at the same content.
    assert!(
        model
            .viewport
            .scroll_to(scroll_position.to_scroll_top(&model), model.height())
    );
    // The reduced content height clamps the restored position to the last viewport.
    assert_eq!(model.viewport.scroll_top().as_f32(), 12.);

    // Halve the original viewport width, leading to twice as many soft-wrapped lines.
    model
        .viewport
        .set_size(vec2f(20., 60.), model.width(), model.height());
    layout_content(&mut model);
    assert_eq!(model.height().as_f32(), 60. + 80. + 24.);

    // Restore the scroll position at the new height.
    assert!(
        model
            .viewport
            .scroll_to(scroll_position.to_scroll_top(&model), model.height())
    );
    // The new scroll position is at the start of the second paragraph.
    assert_eq!(model.viewport.scroll_top().as_f32(), 60.);
}

#[test]
fn test_offset_in_active_selection() {
    let render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let selection_vec: Vec1<RenderedSelection> = vec1![
        RenderedSelection::new(2.into(), 4.into()),
        RenderedSelection::new(6.into(), 8.into()),
        RenderedSelection::new(12.into(), 10.into())
    ];
    let selections = selection_vec.into();
    *render_state.selections.borrow_mut() = selections;

    assert!(render_state.offset_in_active_selection(3.into()));
    assert!(!render_state.offset_in_active_selection(1.into()));
    assert!(render_state.offset_in_active_selection(7.into()));
    assert!(!render_state.offset_in_active_selection(9.into()));
    assert!(!render_state.offset_in_active_selection(2.into()));
    assert!(render_state.offset_in_active_selection(4.into()));
    assert!(!render_state.offset_in_active_selection(10.into()));
    assert!(render_state.offset_in_active_selection(12.into()));
    assert!(render_state.offset_in_active_selection(11.into()));
}

#[test]
fn test_is_selection_head() {
    let render_state =
        RenderState::new_for_test(TEST_STYLES, 10.0.into_pixels(), 10.0.into_pixels());
    let selection_vec: Vec1<RenderedSelection> = vec1![
        RenderedSelection::new(2.into(), 4.into()),
        RenderedSelection::new(6.into(), 8.into()),
        RenderedSelection::new(12.into(), 10.into())
    ];
    let selections = selection_vec.into();
    *render_state.selections.borrow_mut() = selections;

    assert!(render_state.is_selection_head(2.into()));
    assert!(!render_state.is_selection_head(1.into()));
    assert!(!render_state.is_selection_head(4.into()));
    assert!(render_state.is_selection_head(6.into()));
    assert!(render_state.is_selection_head(12.into()));
}

#[test]
fn test_multiselect_autoscroll_bounding_box() {
    // Test that the computation for the autoscroll bounding box work correctly.
    let view_height = 800.0.into_pixels();

    // One selection, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![(vec2f(0., 0.), vec2f(0., 0.))],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(0., 0.), vec2f(0., 0.))
    );

    // One selection, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![(vec2f(100., 100.), vec2f(100., 100.))],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(100., 100.))
    );

    // Two selections, on screen.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 100.), vec2f(100.0, 100.0)),
                (vec2f(200., 200.), vec2f(200., 200.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(200., 200.))
    );

    // Three selections, top two on screen, but the third one is too far to fit.
    // Pick a selection that isn't larger than the viewport
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 100.), vec2f(100.0, 100.0)),
                (vec2f(200., 200.), vec2f(200., 200.)),
                (vec2f(300., 1000.), vec2f(300., 1000.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 100.), vec2f(200., 200.))
    );

    // Three selections, one on screen, so the other two should not be scrolled to.
    // Pick a selection that isn't larger than the viewport
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 700.), vec2f(100.0, 700.0)),
                (vec2f(200., 900.), vec2f(200., 900.)),
                (vec2f(300., 1000.), vec2f(300., 1000.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 700.), vec2f(100., 700.))
    );

    // Three selections, all off screen to the bottom, so we should fit as many as we can.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 1000.), vec2f(100.0, 1000.0)),
                (vec2f(200., 1400.), vec2f(200., 1400.)),
                (vec2f(300., 1900.), vec2f(300., 1900.))
            ],
            view_height,
            0.0.into_pixels(),
        ),
        (vec2f(100., 1000.), vec2f(200., 1400.))
    );

    // Three selections, all off screen to the top, so we should fit as many as we can from the bottom up.
    assert_eq!(
        RenderState::multiselect_autoscroll_bounding_box(
            vec1![
                (vec2f(100., 0.), vec2f(100.0, 0.0)),
                (vec2f(200., 500.), vec2f(200., 500.)),
                (vec2f(300., 1200.), vec2f(300., 1200.))
            ],
            view_height,
            1500.0.into_pixels(),
        ),
        (vec2f(200., 500.), vec2f(300., 1200.))
    );
}

// 18:09:15 [INFO] [warp_editor::render::model] Initial tree:
// -------- 0.00px / 0 characters --------
// Hidden (3067 characters, 87 lines, 20.00px tall)
// -------- 20.00px / 3067 characters --------
// Paragraph (32 characters, 1 lines, 18.20px tall)
// -------- 38.20px / 3099 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 56.40px / 3127 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 74.60px / 3155 characters --------
// Paragraph (37 characters, 1 lines, 18.20px tall)
// -------- 92.80px / 3192 characters --------
// Paragraph (13 characters, 1 lines, 18.20px tall)
// -------- 111.00px / 3205 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 129.20px / 3211 characters --------
// Paragraph (2 characters, 1 lines, 18.20px tall)
// -------- 147.40px / 3213 characters --------
// Hidden (406 characters, 15 lines, 20.00px tall)
// -------- 167.40px / 3619 characters --------
// Paragraph (41 characters, 1 lines, 18.20px tall)
// -------- 185.60px / 3660 characters --------
// Paragraph (73 characters, 1 lines, 18.20px tall)
// -------- 203.80px / 3733 characters --------
// Paragraph (57 characters, 1 lines, 18.20px tall)
// -------- 222.00px / 3790 characters --------
// Paragraph (17 characters, 1 lines, 18.20px tall)
// -------- 240.20px / 3807 characters --------
// Paragraph (36 characters, 1 lines, 18.20px tall)
// -------- 258.40px / 3843 characters --------
// Paragraph (29 characters, 1 lines, 18.20px tall)
// -------- 276.60px / 3872 characters --------
// Temporary Paragraph (0 characters, 0 lines, 18.20px tall)
// -------- 294.80px / 3872 characters --------
// Temporary Paragraph (0 characters, 0 lines, 18.20px tall)
// -------- 313.00px / 3872 characters --------
// Paragraph (10 characters, 1 lines, 18.20px tall)
// -------- 331.20px / 3882 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 349.40px / 3888 characters --------
// Hidden (1 characters, 1 lines, 20.00px tall)
//
// Nothing needs to be changed here. There is no overlapping hidden ranges.
#[test]
fn test_dedupe_hidden_ranges_logged_tree_is_unchanged() {
    // This is a "golden" structure derived from the logs in the prompt.
    // The observed behavior was that `dedupe_hidden_ranges` is a no-op for this tree.

    let mut tree = SumTree::new();

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3066),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    let temporary_paragraph =
        layout_paragraph("\n", &TEST_STYLES, &BufferBlockStyle::PlainText, 80.);
    let temporary_block = BlockItem::TemporaryBlock {
        paragraph_block: ParagraphBlock::new(vec1![temporary_paragraph]),
        text_decoration: Vec::new(),
        decoration: None,
    };
    tree.push(temporary_block.clone());
    tree.push(temporary_block);

    for len in [10usize, 6] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(1),
        CharOffset::from(1),
        BlockLocation::End,
    )));

    let mut hidden_ranges = RangeSet::new();
    hidden_ranges.insert(CharOffset::from(1)..CharOffset::from(3067));
    hidden_ranges.insert(CharOffset::from(3213)..CharOffset::from(3619));
    hidden_ranges.insert(CharOffset::from(3888)..CharOffset::from(3889));

    let initial = tree.describe().to_string();
    let resulting = RenderState::dedupe_hidden_ranges(tree, hidden_ranges)
        .describe()
        .to_string();

    assert_eq!(initial, resulting);
}

// 18:09:14 [INFO] [warp_editor::render::model] Initial tree:
// -------- 0.00px / 0 characters --------
// Hidden (3066 characters, 87 lines, 20.00px tall)
// -------- 20.00px / 3067 characters --------
// Paragraph (32 characters, 1 lines, 18.20px tall)
// -------- 38.20px / 3099 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 56.40px / 3127 characters --------
// Paragraph (28 characters, 1 lines, 18.20px tall)
// -------- 74.60px / 3155 characters --------
// Paragraph (37 characters, 1 lines, 18.20px tall)
// -------- 92.80px / 3192 characters --------
// Paragraph (13 characters, 1 lines, 18.20px tall)
// -------- 111.00px / 3205 characters --------
// Paragraph (6 characters, 1 lines, 18.20px tall)
// -------- 129.20px / 3211 characters --------
// Paragraph (2 characters, 1 lines, 18.20px tall)
// -------- 147.40px / 3213 characters --------
// Hidden (406 characters, 15 lines, 20.00px tall)
// -------- 167.40px / 3619 characters --------
// Paragraph (41 characters, 1 lines, 18.20px tall)
// -------- 185.60px / 3660 characters --------
// Paragraph (73 characters, 1 lines, 18.20px tall)
// -------- 203.80px / 3733 characters --------
// Paragraph (57 characters, 1 lines, 18.20px tall)
// -------- 222.00px / 3790 characters --------
// Paragraph (17 characters, 1 lines, 18.20px tall)
// -------- 240.20px / 3807 characters --------
// Paragraph (36 characters, 1 lines, 18.20px tall)
// -------- 258.40px / 3843 characters --------
// Paragraph (29 characters, 1 lines, 18.20px tall)
// -------- 276.60px / 3872 characters --------
// Hidden (1 characters, 1 lines, 20.00px tall)
// -------- 296.60px / 3873 characters --------
// Hidden (1944 characters, 45 lines, 20.00px tall)
//
// The last two hidden sections should be collapsed.
#[test]
fn test_dedupe_hidden_ranges_merges_adjacent_hidden_blocks() {
    let mut tree = SumTree::new();

    // Pushing a hidden range that actually exceed what is expected from the canonical range.
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3067),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        tree.push(mock_paragraph(18.2, 0., len));
    }

    // Two adjacent hidden blocks.
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(1),
        CharOffset::from(1),
        BlockLocation::Middle,
    )));
    tree.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(45),
        CharOffset::from(1944),
        BlockLocation::End,
    )));

    let mut hidden_ranges = RangeSet::new();
    hidden_ranges.insert(CharOffset::from(1)..CharOffset::from(3067));
    hidden_ranges.insert(CharOffset::from(3213)..CharOffset::from(3619));

    // Covers both adjacent hidden blocks (3872 + 1 + 1944 = 5817 total content length).
    hidden_ranges.insert(CharOffset::from(3872)..CharOffset::from(5818));

    let resulting = RenderState::dedupe_hidden_ranges(tree, hidden_ranges);

    let mut expected = SumTree::new();

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(87),
        CharOffset::from(3066),
        BlockLocation::Start,
    )));

    for len in [32usize, 28, 28, 37, 13, 6, 2] {
        expected.push(mock_paragraph(18.2, 0., len));
    }

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(15),
        CharOffset::from(406),
        BlockLocation::Middle,
    )));

    for len in [41usize, 73, 57, 17, 36, 29] {
        expected.push(mock_paragraph(18.2, 0., len));
    }

    expected.push(BlockItem::Hidden(HiddenBlockConfig::new(
        LineCount(46),
        CharOffset::from(1946),
        BlockLocation::End,
    )));

    assert_eq!(
        expected.describe().to_string(),
        resulting.describe().to_string()
    );
}

#[allow(clippy::single_range_in_vec_init)]
fn make_test_cell_layout() -> CellLayout {
    CellLayout {
        line_heights: vec![20.0],
        line_y_offsets: vec![0.0],
        line_char_ranges: vec![CharOffset::from(0)..CharOffset::from(3)],
        line_widths: vec![30.0],
        line_caret_positions: vec![vec![
            warpui::text_layout::CaretPosition {
                position_in_line: 0.0,
                start_offset: 0,
                last_offset: 0,
            },
            warpui::text_layout::CaretPosition {
                position_in_line: 10.0,
                start_offset: 1,
                last_offset: 1,
            },
            warpui::text_layout::CaretPosition {
                position_in_line: 20.0,
                start_offset: 2,
                last_offset: 2,
            },
        ]],
    }
}

#[test]
fn test_line_at_char_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.line_at_char_offset(CharOffset::from(0)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(1)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(2)), Some(0));
    assert_eq!(layout.line_at_char_offset(CharOffset::from(5)), Some(0));
}

#[test]
fn test_x_for_char_in_line() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.x_for_char_in_line(0, 0), 0.0);
    assert_eq!(layout.x_for_char_in_line(0, 1), 10.0);
    assert_eq!(layout.x_for_char_in_line(0, 2), 20.0);
    assert_eq!(layout.x_for_char_in_line(0, 3), 30.0);
}

#[test]
fn test_line_at_y_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.line_at_y_offset(0.0), 0);
    assert_eq!(layout.line_at_y_offset(10.0), 0);
    assert_eq!(layout.line_at_y_offset(19.9), 0);
    assert_eq!(layout.line_at_y_offset(20.0), 0);
}

#[test]
fn test_char_at_x_in_line_at_zero() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 0.0), CharOffset::from(0));
}

#[test]
fn test_char_at_x_in_line_at_small_x() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 1.0), CharOffset::from(0));
    assert_eq!(layout.char_at_x_in_line(0, 4.0), CharOffset::from(0));
}

#[test]
fn test_char_at_x_in_line_at_boundary() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 5.0), CharOffset::from(1));
    assert_eq!(layout.char_at_x_in_line(0, 10.0), CharOffset::from(1));
}

#[test]
fn test_char_at_x_in_line_near_line_end_maps_to_end_offset() {
    let layout = make_test_cell_layout();
    assert_eq!(layout.char_at_x_in_line(0, 25.0), CharOffset::from(3));
}

fn make_test_laid_out_table() -> LaidOutTable {
    let source = "aaa\tbbb\nccc\tddd\n";
    let table = FormattedTable::from_internal_format(source);
    let cell_offset_maps = table_cell_offset_maps(&table, source);
    let offset_map = table_offset_map::TableOffsetMap::new(
        cell_offset_maps
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| cell.source_length().as_usize())
                    .collect()
            })
            .collect(),
    );
    let content_length = offset_map.total_length();
    let cell_layout = make_test_cell_layout();
    let cell_frame = Arc::new(TextFrame::mock("aaa"));
    LaidOutTable {
        table,
        config: TableBlockConfig {
            width: 60.0.into_pixels(),
            spacing: DEFAULT_BLOCK_SPACINGS.text,
            style: TableStyle {
                border_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                header_background: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                cell_background: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                alternate_row_background: None,
                text_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                header_text_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                scrollbar_nonactive_thumb_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                scrollbar_active_thumb_color: ColorU {
                    r: 0,
                    g: 0,
                    b: 0,
                    a: 255,
                },
                font_family: FamilyId(0),
                font_size: 10.0,
                cell_padding: 0.0,
                outer_border: true,
                column_dividers: true,
                row_dividers: true,
            },
        },
        row_heights: vec![20.0.into_pixels(), 20.0.into_pixels()],
        column_widths: vec![30.0.into_pixels(), 30.0.into_pixels()],
        total_height: 40.0.into_pixels(),
        offset_map,
        content_length,
        cell_offset_maps,
        row_y_offsets: vec![0.0, 20.0, 40.0],
        col_x_offsets: vec![0.0, 30.0, 60.0],
        cell_text_frames: vec![
            vec![cell_frame.clone(), cell_frame.clone()],
            vec![cell_frame.clone(), cell_frame],
        ],
        cell_layouts: vec![
            vec![cell_layout.clone(), cell_layout.clone()],
            vec![cell_layout.clone(), cell_layout],
        ],
        cell_links: vec![vec![vec![], vec![]], vec![vec![], vec![]]],
        scroll_left: Cell::new(Pixels::zero()),
        scrollbar_interaction_state: Default::default(),
        horizontal_scroll_allowed: true,
    }
}

#[test]
fn test_coordinate_to_offset() {
    let table = make_test_laid_out_table();
    assert_eq!(table.coordinate_to_offset(0.0, 0.0), CharOffset::from(0));
    assert_eq!(table.coordinate_to_offset(10.0, 0.0), CharOffset::from(1));
    assert_eq!(table.coordinate_to_offset(30.0, 0.0), CharOffset::from(4));
    assert_eq!(table.coordinate_to_offset(0.0, 20.0), CharOffset::from(8));
}

#[test]
fn test_coordinate_to_offset_near_cell_line_end_maps_to_cell_end() {
    let table = make_test_laid_out_table();
    assert_eq!(table.coordinate_to_offset(25.0, 0.0), CharOffset::from(3));
}

#[test]
fn test_reveal_offset_scrolls_table_character_into_view() {
    let table = make_test_laid_out_table();
    assert_eq!(table.scroll_left(), Pixels::zero());
    assert!(table.reveal_offset(CharOffset::from(5), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), 28.0.into_pixels());
}

#[test]
fn test_disabled_horizontal_scroll_returns_full_viewport_width() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert_eq!(table.viewport_width(30.0.into_pixels()), table.width());
    assert_eq!(table.max_scroll_left(30.0.into_pixels()), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_reports_zero_scroll_left() {
    let mut table = make_test_laid_out_table();
    table.scroll_left.set(15.0.into_pixels());
    table.horizontal_scroll_allowed = false;

    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_set_scroll_left_is_noop() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert!(!table.set_scroll_left(20.0.into_pixels(), 30.0.into_pixels()));
    assert!(!table.scroll_horizontally(10.0.into_pixels(), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_disabled_horizontal_scroll_reveal_offset_is_noop() {
    let mut table = make_test_laid_out_table();
    table.horizontal_scroll_allowed = false;

    assert!(!table.reveal_offset(CharOffset::from(5), 30.0.into_pixels()));
    assert_eq!(table.scroll_left(), Pixels::zero());
}

#[test]
fn test_link_at_offset_uses_cached_cell_links() {
    let mut table = make_test_laid_out_table();
    table.table = FormattedTable {
        headers: vec![
            vec![
                FormattedTextFragment::plain_text("a"),
                FormattedTextFragment {
                    text: "bc".into(),
                    styles: FormattedTextStyles {
                        hyperlink: Some(Hyperlink::Url("https://warp.dev".into())),
                        ..Default::default()
                    },
                },
            ],
            vec![FormattedTextFragment::plain_text("bbb")],
        ],
        alignments: vec![],
        rows: vec![vec![
            vec![FormattedTextFragment::plain_text("ccc")],
            vec![FormattedTextFragment::plain_text("ddd")],
        ]],
    };
    table.cell_links = vec![
        vec![
            vec![ParsedUrl::new(1..3, "https://warp.dev".into())],
            vec![],
        ],
        vec![vec![], vec![]],
    ];

    assert_eq!(
        table.link_at_offset(CharOffset::from(1)),
        Some("https://warp.dev".into())
    );
    assert_eq!(
        table.link_at_offset(CharOffset::from(2)),
        Some("https://warp.dev".into())
    );
    assert_eq!(table.link_at_offset(CharOffset::from(0)), None);
    assert_eq!(table.link_at_offset(CharOffset::from(3)), None);
}
