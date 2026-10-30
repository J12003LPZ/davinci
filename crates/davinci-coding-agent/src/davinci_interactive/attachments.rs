//! Images pasted into the composer (`ctrl+v` / `alt+v`), as Claude Code
//! shows them: each paste inserts an `[Image #N]` chip where the cursor is,
//! and an image goes with the prompt only while its chip is still in the
//! text. Deleting the chip, or clearing the draft, drops the image.

use davinci_ai::MessageContent;

/// The chip for the `number`th image pasted since the last prompt.
pub(super) fn chip(number: usize) -> String {
    format!("[Image #{number}]")
}

/// The chip the next pasted image gets.
pub(super) fn next_chip(images: &[MessageContent]) -> String {
    chip(images.len() + 1)
}

/// The pasted images whose chip is still in `text`, in paste order.
pub(super) fn retained(text: &str, images: &[MessageContent]) -> Vec<MessageContent> {
    images
        .iter()
        .enumerate()
        .filter(|(index, _)| text.contains(&chip(index + 1)))
        .map(|(_, image)| image.clone())
        .collect()
}

/// The chips of the pasted images that go with `text`, in paste order: what
/// the transcript lists under the prompt as `⎿ [Image #1]`.
pub(super) fn chips_in(text: &str, images: &[MessageContent]) -> Vec<String> {
    (1..=images.len())
        .map(chip)
        .filter(|chip| text.contains(chip))
        .collect()
}

/// Whether `text` still shows a chip of one of `images`.
pub(super) fn any_chip(text: &str, images: &[MessageContent]) -> bool {
    !retained(text, images).is_empty()
}

/// The chip as inserted at the cursor: a space before it when it would
/// otherwise touch the word before, and one after so typing continues.
pub(super) fn spaced_chip(chip: &str, before_cursor: Option<char>) -> String {
    match before_cursor {
        Some(c) if !c.is_whitespace() => format!(" {chip} "),
        _ => format!("{chip} "),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(tag: &str) -> MessageContent {
        MessageContent::Image {
            data: tag.into(),
            mime_type: "image/png".into(),
        }
    }

    #[test]
    fn chips_number_images_in_paste_order() {
        assert_eq!(next_chip(&[]), "[Image #1]");
        assert_eq!(next_chip(&[image("a"), image("b")]), "[Image #3]");
    }

    #[test]
    fn only_images_whose_chip_survives_are_sent() {
        let images = [image("a"), image("b"), image("c")];
        let kept = retained("compare [Image #1] with [Image #3]", &images);
        assert_eq!(kept, vec![image("a"), image("c")]);
        assert!(retained("", &images).is_empty());
        // A partly deleted chip no longer counts.
        assert!(retained("[Image #2", &images).is_empty());
    }

    #[test]
    fn the_transcript_lists_one_chip_per_image_still_in_the_text() {
        let images = [image("a"), image("b"), image("c")];
        assert_eq!(
            chips_in("compare [Image #1] with [Image #3]", &images),
            ["[Image #1]", "[Image #3]"]
        );
        assert!(chips_in("no chips here", &images).is_empty());
        assert!(chips_in("[Image #1]", &[]).is_empty());
        // `#1` is not `#10`.
        assert_eq!(chips_in("[Image #10]", &images), Vec::<String>::new());
    }

    #[test]
    fn a_chip_is_spaced_only_from_the_word_before_the_cursor() {
        assert_eq!(spaced_chip("[Image #1]", None), "[Image #1] ");
        assert_eq!(spaced_chip("[Image #1]", Some(' ')), "[Image #1] ");
        assert_eq!(spaced_chip("[Image #1]", Some('o')), " [Image #1] ");
        assert_eq!(spaced_chip("[Image #2]", Some('\n')), "[Image #2] ");
    }

    #[test]
    fn any_chip_sees_only_chips_of_held_images() {
        let images = [image("a")];
        assert!(any_chip("look [Image #1]", &images));
        assert!(!any_chip("look [Image #2]", &images));
        assert!(!any_chip("[Image #1]", &[]));
    }
}
