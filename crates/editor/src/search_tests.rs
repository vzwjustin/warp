use warpui::{App, ModelHandle};

use super::Searcher;
use crate::content::buffer::{
    AutoScrollBehavior, Buffer, BufferEditAction, BufferSelectAction, EditOrigin,
};
use crate::content::find::{Match, SearchResults};
use crate::content::selection_model::BufferSelectionModel;
use crate::content::text::IndentBehavior;

#[test]
fn test_literal_search() {
    App::test((), |mut app| async move {
        let (content, selection) = buffer(&mut app, "abc def abc def");
        let search = searcher(&mut app, content, selection);

        search
            .update(&mut app, |search, ctx| {
                search.set_query("abc", ctx);
                search.search_finished(ctx)
            })
            .await;

        search.read(&app, |search, _| {
            assert_eq!(search.match_count(), 2);
            assert_eq!(
                search.results,
                Some(SearchResults {
                    matches: vec![
                        Match {
                            start: 1.into(),
                            end: 4.into()
                        },
                        Match {
                            start: 9.into(),
                            end: 12.into()
                        }
                    ]
                })
            )
        })
    });
}

#[test]
fn test_navigate_search_results() {
    App::test((), |mut app| async move {
        let (content, selection) = buffer(&mut app, "abc def abc def");
        let search = searcher(&mut app, content, selection);

        search
            .update(&mut app, |search, ctx| {
                search.set_auto_select(true);
                search.set_query("abc", ctx);
                search.search_finished(ctx)
            })
            .await;

        search.update(&mut app, |search, ctx| {
            assert_eq!(search.match_count(), 2);
            // With auto-selection, the first match should be selected (cursor is at start)
            assert_eq!(search.selected_match(), Some(0));

            // Move to the next result.
            search.select_next_result(ctx);
            assert_eq!(search.selected_match(), Some(1));

            // Keep moving until we wrap around.
            search.select_next_result(ctx);
            assert_eq!(search.selected_match(), Some(0));

            // Now do the same in the other direction.
            search.select_previous_result(ctx);
            assert_eq!(search.selected_match(), Some(1));

            search.select_previous_result(ctx);
            assert_eq!(search.selected_match(), Some(0));

            // Starting from no selection, select the last result.
            search.selected_result = None;
            search.select_previous_result(ctx);
            assert_eq!(search.selected_match(), Some(1));
        })
    });
}

#[test]
fn test_edit_results_valid() {
    App::test((), |mut app| async move {
        let (content, content_selection) = buffer(&mut app, "a git command");
        let search = searcher(&mut app, content.clone(), content_selection.clone());

        search
            .update(&mut app, |search, ctx| {
                search.set_auto_select(true);
                search.set_regex(true, ctx);
                search.set_query(r"g\w+t", ctx);
                search.search_finished(ctx)
            })
            .await;

        search.update(&mut app, |search, ctx| {
            search.select_next_result(ctx);
            assert_eq!(search.selected_match(), Some(0));
        });

        // Insert some text before the match.
        content.update(&mut app, |buffer, ctx| {
            buffer.update_selection(
                content_selection.clone(),
                BufferSelectAction::AddCursorAt {
                    offset: 2.into(),
                    clear_selections: true,
                },
                AutoScrollBehavior::Selection,
                ctx,
            );
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "sdf",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                content_selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>asdf git command");
        });

        search
            .update(&mut app, |search, ctx| search.search_finished(ctx))
            .await;

        // When the search finishes, we should have the same result.
        assert_eq!(
            search.read(&app, |search, _| search.selected_match()),
            Some(0)
        );

        // Insert a new match before the selected one, which should move it back.
        content.update(&mut app, |buffer, ctx| {
            buffer.update_content(
                BufferEditAction::Insert {
                    text: "got",
                    style: Default::default(),
                    override_text_style: None,
                },
                EditOrigin::UserTyped,
                content_selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>asdfgot git command");
        });

        search
            .update(&mut app, |search, ctx| search.search_finished(ctx))
            .await;

        assert_eq!(
            search.read(&app, |search, _| search.selected_match()),
            Some(1)
        );
    })
}

#[test]
fn test_edit_results_invalid() {
    App::test((), |mut app| async move {
        let (content, content_selection) = buffer(&mut app, "a git command for git");
        let search = searcher(&mut app, content.clone(), content_selection.clone());

        search
            .update(&mut app, |search, ctx| {
                search.set_auto_select(true);
                search.set_regex(true, ctx);
                search.set_query(r"g\w+t", ctx);
                search.search_finished(ctx)
            })
            .await;

        search.update(&mut app, |search, ctx| {
            // Move to the second match
            search.select_next_result(ctx);
            assert_eq!(search.selected_match(), Some(1));
        });

        // Invalidate the match
        content.update(&mut app, |buffer, ctx| {
            buffer.update_selection(
                content_selection.clone(),
                BufferSelectAction::AddCursorAt {
                    offset: 6.into(),
                    clear_selections: true,
                },
                AutoScrollBehavior::Selection,
                ctx,
            );
            buffer.update_content(
                BufferEditAction::Backspace,
                EditOrigin::UserTyped,
                content_selection.clone(),
                ctx,
            );
            assert_eq!(buffer.debug(), "<text>a gi command for git");
        });

        search
            .update(&mut app, |search, ctx| search.search_finished(ctx))
            .await;

        // When the search finishes, auto-selection should pick the remaining match.
        search.read(&app, |search, _| {
            assert_eq!(search.selected_match(), Some(0));
            assert_eq!(search.match_count(), 1);
        });
    })
}
/// Create a new buffer model with the given Markdown.
fn buffer(
    app: &mut App,
    markdown: &str,
) -> (ModelHandle<Buffer>, ModelHandle<BufferSelectionModel>) {
    let buffer = app.add_model(|_| Buffer::new(Box::new(|_, _| IndentBehavior::Ignore)));
    let selection = app.add_model(|_| BufferSelectionModel::new(buffer.clone()));

    buffer.update(app, |buffer, ctx| {
        *buffer = Buffer::from_markdown(
            markdown,
            None,
            Box::new(|_, _| IndentBehavior::Ignore),
            selection.clone(),
            ctx,
        );
    });

    (buffer, selection)
}

fn searcher(
    app: &mut App,
    buffer: ModelHandle<Buffer>,
    selection: ModelHandle<BufferSelectionModel>,
) -> ModelHandle<Searcher> {
    app.add_model(|ctx| Searcher::new(buffer, selection, ctx))
}

// ── CLD-558 characterization test ─────────────────────────────────────────────
//
// Pins the coordinate conversion in `result_decorations`: search `Match`
// offsets are 1-indexed (buffer space) and `Decoration` offsets are 0-indexed
// (render-model space).  The `−1` on both bounds converts between the two.

/// CLD-558 characterization: search result decorations use render (0-indexed) offsets.
///
/// For input "abc def abc def" the two matches for "abc" are at buffer offsets
/// 1..4 and 9..12 (1-indexed).  After the `−1` conversion, the decorations
/// must cover render offsets 0..3 and 8..11 (0-indexed).
#[test]
fn test_cld558_search_decorations_use_render_offsets() {
    use crate::render::model::Decoration;
    App::test((), |mut app| async move {
        let (content, selection) = buffer(&mut app, "abc def abc def");
        let search = searcher(&mut app, content, selection);

        search
            .update(&mut app, |search, ctx| {
                search.set_query("abc", ctx);
                search.search_finished(ctx)
            })
            .await;

        search.read(&app, |search, _| {
            // Verify the raw match offsets are in 1-indexed buffer space.
            let results = search.results().expect("search should have results");
            assert_eq!(
                results.matches,
                vec![
                    Match {
                        start: 1.into(),
                        end: 4.into()
                    },
                    Match {
                        start: 9.into(),
                        end: 12.into()
                    },
                ],
                "search matches should be 1-indexed buffer offsets"
            );

            // Verify that decorations subtract 1, yielding 0-indexed render offsets.
            let decorations: Vec<Decoration> = search.result_decorations();
            assert_eq!(decorations.len(), 2, "should have two decorations");

            // First match "abc" at buffer 1..4 → render 0..3.
            assert_eq!(
                (decorations[0].start, decorations[0].end),
                (0.into(), 3.into()),
                "first decoration should be at render offsets 0..3"
            );

            // Second match "abc" at buffer 9..12 → render 8..11.
            assert_eq!(
                (decorations[1].start, decorations[1].end),
                (8.into(), 11.into()),
                "second decoration should be at render offsets 8..11"
            );
        });
    });
}
