use super::*;

fn state() -> EditorState {
    EditorState::for_test(
        AnnotationDocument::new(PhysicalSize::new(200, 100)),
        PhysicalSize::new(200, 100),
    )
}

fn rect(x: i32, y: i32, width: u32, height: u32) -> Shape {
    Shape::Rect(PhysicalRect::new(x, y, width, height))
}

#[test]
fn rect_style_uses_tool_defaults_and_scaled_radius() {
    let mut state = state();
    state.tool = Tool::Rect;
    state.rect_stroke_width = 7.0;
    state.corner_radius = 8.0;
    state.ui_scale = 2.0;
    state.rect_filled = true;

    let style = state.effective_style();
    assert_eq!(style.stroke_width, 7.0);
    assert_eq!(style.corner_radius, 16.0);
    assert!(style.filled);
    assert_eq!(style.color & 0xFF, 0xFF);
}

#[test]
fn ellipse_style_reads_its_own_fill_default() {
    let mut state = state();
    state.tool = Tool::Rect;
    state.shape_ellipse = true;
    state.ellipse_filled = true;
    state.rect_filled = false;

    assert!(state.effective_style().filled);

    state.ellipse_filled = false;
    assert!(!state.effective_style().filled);
}

#[test]
fn highlighter_forces_translucent_wash_and_fixed_width() {
    let mut state = state();
    state.tool = Tool::Highlighter;
    state.style.stroke_width = 3.0;

    let style = state.effective_style();
    assert_eq!(style.stroke_width, 12.0);
    assert_eq!(style.color & 0xFF, core_annotate::HIGHLIGHTER_ALPHA);
}

#[test]
fn arrow_style_uses_the_arrow_width_and_heads() {
    let mut state = state();
    state.tool = Tool::Arrow;
    state.arrow_stroke_width = 5.0;
    state.rect_stroke_width = 2.0;
    state.arrow_start = true;
    state.arrow_end = false;

    let style = state.effective_style();
    assert_eq!(style.stroke_width, 5.0);
    assert!(style.arrow_start);
    assert!(!style.arrow_end);
}

#[test]
fn text_and_spotlight_styles_carry_their_options() {
    let mut state = state();
    state.tool = Tool::Text;
    state.font_size = 28.0;
    state.text_background = true;
    state.text_outline = true;
    state.text_alignment = TextAlignment::Center;

    let style = state.effective_style();
    assert_eq!(style.font_size, 28.0);
    assert!(style.text_background);
    assert!(style.text_outline);
    assert_eq!(style.text_alignment, TextAlignment::Center);

    state.tool = Tool::Spotlight;
    state.spotlight_intensity = 60;
    state.spotlight_radius = 10.0;
    state.ui_scale = 2.0;
    let style = state.effective_style();
    assert_eq!(style.spotlight_intensity, 60);
    assert_eq!(style.spotlight_radius, 20.0);
}

#[test]
fn selecting_a_shape_restores_its_tool_defaults() {
    let mut state = state();
    state.ui_scale = 2.0;
    state.tool = Tool::Rect;
    let id = state.document.add(
        rect(10, 10, 40, 30),
        AnnotationStyle {
            corner_radius: 20.0,
            filled: true,
            stroke_width: 7.0,
            ..AnnotationStyle::default()
        },
    );

    state.select_annotation(Some(id));
    assert_eq!(state.corner_radius, 10.0);
    assert!(state.rect_filled);
    assert_eq!(state.style.stroke_width, 7.0);

    state.select_annotation(None);
    assert_eq!(state.style.stroke_width, state.rect_stroke_width);
}

#[test]
fn a_move_gesture_records_one_undo_step() {
    let mut state = state();
    let original = rect(10, 10, 20, 20);
    let id = state
        .document
        .add(original.clone(), AnnotationStyle::default());
    state.move_draft = Some(MoveDraft {
        id,
        start: PhysicalPoint::new(10, 10),
        current: PhysicalPoint::new(10, 10),
        original,
        recorded: false,
    });

    assert!(!state.update_move(PhysicalPoint::new(10, 10)));
    assert!(state.update_move(PhysicalPoint::new(14, 18)));
    assert_eq!(
        state.document.object(id).unwrap().shape.bounds().origin,
        PhysicalPoint::new(14, 18)
    );
    assert!(state.document.can_undo());

    assert!(state.update_move(PhysicalPoint::new(20, 20)));
    assert!(state.document.undo());
    assert_eq!(
        state.document.object(id).unwrap().shape.bounds().origin,
        PhysicalPoint::new(10, 10)
    );
}

#[test]
fn resizing_a_number_keeps_it_square() {
    let mut state = state();
    let original = Shape::Number {
        bounds: PhysicalRect::new(10, 10, 32, 32),
        value: 1,
    };
    let id = state
        .document
        .add(original.clone(), AnnotationStyle::default());
    state.resize_draft = Some(ResizeDraft {
        id,
        handle: ResizeHandle::Se,
        start: PhysicalPoint::new(42, 42),
        current: PhysicalPoint::new(42, 42),
        original,
        recorded: false,
    });

    assert!(state.update_resize(PhysicalPoint::new(62, 52)));
    let Shape::Number { bounds, value } = &state.document.object(id).unwrap().shape else {
        panic!("expected a number shape");
    };
    assert_eq!(bounds.size.width, bounds.size.height);
    assert_eq!(bounds.size.width, 52);
    assert_eq!(*value, 1);
}

#[test]
fn rect_drafts_are_normalized_and_minimum_sized() {
    let draft = Draft {
        tool: Tool::Rect,
        start: PhysicalPoint::new(50, 30),
        current: PhysicalPoint::new(10, 10),
        shift: false,
        ellipse: false,
    };
    let shape = finalize_shape(&draft, draft.current).expect("rect draft");
    assert_eq!(shape.bounds(), PhysicalRect::new(10, 10, 40, 20));

    let tiny = Draft {
        current: PhysicalPoint::new(51, 31),
        ..draft
    };
    assert!(finalize_shape(&tiny, tiny.current).is_none());
}

#[test]
fn shift_drag_constrains_rectangles_to_squares() {
    let draft = Draft {
        tool: Tool::Rect,
        start: PhysicalPoint::new(10, 10),
        current: PhysicalPoint::new(50, 30),
        shift: true,
        ellipse: false,
    };
    let shape = finalize_shape(&draft, draft.current).expect("square draft");
    assert_eq!(shape.bounds().size.width, 40);
    assert_eq!(shape.bounds().size.height, 40);
}

#[test]
fn ime_commit_replaces_the_in_flight_composition() {
    let mut edit = TextEdit::start(
        PhysicalPoint::new(0, 0),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    );
    edit.replace(None, "cafe", true);
    assert_eq!(edit.content, "cafe");
    assert!(edit.marked.is_some());

    edit.replace(None, "café", false);
    assert_eq!(edit.content, "café");
    assert!(edit.marked.is_none());
}

#[test]
fn utf16_offsets_round_trip_through_multibyte_text() {
    let mut edit = TextEdit::start(
        PhysicalPoint::new(0, 0),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    );
    edit.content = "a😀b".to_owned();
    assert_eq!(edit.offset_from_utf16(1), 1);
    assert_eq!(edit.offset_from_utf16(3), 5);
    assert_eq!(edit.offset_to_utf16(1), 1);
    assert_eq!(edit.offset_to_utf16(5), 3);
    assert_eq!(edit.range_from_utf16(&(1..3)), 1..5);
    assert_eq!(edit.range_to_utf16(&(1..5)), 1..3);
}

#[test]
fn commit_text_adds_and_updates_a_text_shape() {
    let mut state = state();
    state.text_edit = Some(TextEdit::start(
        PhysicalPoint::new(10, 10),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    ));
    state
        .text_edit
        .as_mut()
        .unwrap()
        .replace(None, "hello", false);
    assert!(state.commit_text());
    assert_eq!(state.document.objects().len(), 1);

    let Shape::Text { text, .. } = &state.document.objects()[0].shape else {
        panic!("expected a text shape");
    };
    assert_eq!(text, "hello");

    let id = state.document.objects()[0].id;
    let bounds = state.document.objects()[0].shape.bounds();
    let style = state.document.objects()[0].style;
    state.text_edit = Some(TextEdit::edit_existing(
        id,
        bounds,
        "hello".to_owned(),
        style,
    ));
    state
        .text_edit
        .as_mut()
        .unwrap()
        .replace(Some(0..5), "bye", false);
    assert!(state.commit_text());
    assert_eq!(state.document.objects().len(), 1);
    let Shape::Text { text, .. } = &state.document.object(id).unwrap().shape else {
        panic!("expected a text shape");
    };
    assert_eq!(text, "bye");
}

#[test]
fn committing_an_empty_edit_removes_the_target() {
    let mut state = state();
    let style = AnnotationStyle::default();
    let bounds = PhysicalRect::new(10, 10, 40, 20);
    let id = state.document.add(
        Shape::Text {
            bounds,
            text: "old".to_owned(),
        },
        style,
    );
    state.text_edit = Some(TextEdit::edit_existing(id, bounds, "old".to_owned(), style));
    state.text_edit.as_mut().unwrap().content.clear();

    assert!(state.commit_text());
    assert!(state.document.object(id).is_none());
}

#[test]
fn backspace_and_delete_edit_the_text_buffer() {
    let mut state = state();
    let mut edit = TextEdit::start(
        PhysicalPoint::new(0, 0),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    );
    edit.content = "abcd".to_owned();
    edit.cursor = 4;
    edit.anchor = 2;
    state.text_edit = Some(edit);

    assert!(state.backspace_text());
    assert_eq!(state.text_edit.as_ref().unwrap().content, "ab");

    let edit = state.text_edit.as_mut().unwrap();
    edit.cursor = 1;
    edit.anchor = 1;
    assert!(state.delete_text());
    assert_eq!(state.text_edit.as_ref().unwrap().content, "a");
}

#[test]
fn select_all_and_move_cursor_keep_utf8_boundaries() {
    let mut state = state();
    let mut edit = TextEdit::start(
        PhysicalPoint::new(0, 0),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    );
    edit.content = "aéb".to_owned();
    state.text_edit = Some(edit);

    assert!(state.select_all_text());
    assert_eq!(state.text_edit.as_ref().unwrap().selection(), 0..4);
    assert!(state.move_text_cursor(2, false));
    assert_eq!(state.text_edit.as_ref().unwrap().cursor, 1);
    assert_eq!(state.text_edit.as_ref().unwrap().anchor, 1);
}

#[test]
fn edit_text_at_reuses_the_existing_text_shape() {
    let mut state = state();
    let style = AnnotationStyle::default();
    let id = state.document.add(
        Shape::Text {
            bounds: PhysicalRect::new(10, 10, 100, 20),
            text: "hello".to_owned(),
        },
        style,
    );

    assert!(state.edit_text_at(PhysicalPoint::new(20, 15)));
    let edit = state.text_edit.as_ref().expect("text edit");
    assert_eq!(edit.target, Some(id));
    assert_eq!(edit.content, "hello");
    assert_eq!(state.document.selected(), Some(id));
}

#[test]
fn finish_polyline_adds_the_pending_point() {
    let mut state = state();
    state.tool = Tool::Arrow;
    state.polyline_draft = Some(PolylineDraft {
        points: vec![PhysicalPoint::new(0, 0), PhysicalPoint::new(10, 10)],
        current: PhysicalPoint::new(20, 20),
    });

    assert!(state.finish_polyline());
    let Shape::Polyline { points } = &state.document.objects()[0].shape else {
        panic!("expected a polyline");
    };
    assert_eq!(points.len(), 3);
    assert!(state.polyline_draft.is_none());
}

#[test]
fn finish_resize_commits_and_clears_the_draft() {
    let mut state = state();
    let original = Shape::Number {
        bounds: PhysicalRect::new(10, 10, 32, 32),
        value: 1,
    };
    let id = state
        .document
        .add(original.clone(), AnnotationStyle::default());
    state.resize_draft = Some(ResizeDraft {
        id,
        handle: ResizeHandle::Se,
        start: PhysicalPoint::new(42, 42),
        current: PhysicalPoint::new(42, 42),
        original,
        recorded: false,
    });

    assert!(state.finish_resize(PhysicalPoint::new(62, 52)));
    assert!(state.resize_draft.is_none());
    let Shape::Number { bounds, .. } = &state.document.object(id).unwrap().shape else {
        panic!("expected a number shape");
    };
    assert_eq!(bounds.size.width, bounds.size.height);
    assert_eq!(bounds.size.width, 52);
}

#[test]
fn cancel_text_reports_whether_an_edit_was_open() {
    let mut state = state();
    assert!(!state.cancel_text());

    state.text_edit = Some(TextEdit::start(
        PhysicalPoint::new(0, 0),
        PhysicalSize::new(200, 100),
        AnnotationStyle::default(),
    ));
    assert!(state.cancel_text());
    assert!(state.text_edit.is_none());
}

fn snapshot(tag: u8) -> EditorSnapshot {
    let frame = core_types::RgbaFrame::from_pixels(1, 1, vec![tag, 0, 0, 255]).expect("frame");
    EditorSnapshot {
        frame: std::sync::Arc::new(frame),
        image_size: PhysicalSize::new(1, 1),
        document: AnnotationDocument::new(PhysicalSize::new(1, 1)),
    }
}

#[test]
fn canvas_history_keeps_three_steps_and_clears_redo() {
    let mut state = state();
    for tag in 0..4 {
        state.record_canvas_snapshot(snapshot(tag));
    }
    assert_eq!(state.canvas_undo.len(), 3);
    assert_eq!(state.canvas_undo[0].frame.pixel(0, 0).unwrap()[0], 1);

    let restored = state.take_undo_snapshot(snapshot(9)).expect("undo step");
    assert_eq!(restored.frame.pixel(0, 0).unwrap()[0], 3);
    assert_eq!(state.canvas_redo.len(), 1);

    state.record_canvas_snapshot(snapshot(8));
    assert!(state.canvas_redo.is_empty());
    assert_eq!(state.canvas_undo.len(), 3);
}

#[test]
fn redo_moves_the_current_canvas_back_to_undo() {
    let mut state = state();
    state.record_canvas_snapshot(snapshot(1));
    let undone = state.take_undo_snapshot(snapshot(2)).expect("undo step");
    assert_eq!(undone.frame.pixel(0, 0).unwrap()[0], 1);

    let redone = state.take_redo_snapshot(snapshot(1)).expect("redo step");
    assert_eq!(redone.frame.pixel(0, 0).unwrap()[0], 2);
    assert_eq!(state.canvas_undo.len(), 1);
}
