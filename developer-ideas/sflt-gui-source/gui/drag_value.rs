#![allow(clippy::needless_pass_by_value)] // False positives with `impl ToString`
#![allow(dead_code)]

use std::{cmp::Ordering, ops::RangeInclusive};

use egui::{
    emath, Button, NumExt, Response, RichText, Sense,
    TextWrapMode, Ui, Widget, WidgetInfo,
};

use crate::gui::MINUS_CHAR_STR;


// ----------------------------------------------------------------------------

type NumFormatter<'a> = Box<dyn 'a + Fn(f64, RangeInclusive<usize>) -> String>;
type NumParser<'a> = Box<dyn 'a + Fn(&str) -> Option<f64>>;

// ----------------------------------------------------------------------------

/// Combined into one function (rather than two) to make it easier
/// for the borrow checker.
type GetSetValue<'a> = Box<dyn 'a + FnMut(Option<f64>) -> f64>;

fn get(get_set_value: &mut GetSetValue<'_>) -> f64 {
    (get_set_value)(None)
}

fn set(get_set_value: &mut GetSetValue<'_>, value: f64) {
    (get_set_value)(Some(value));
}

/// A numeric value that you can change by dragging the number. More compact than a [`crate::Slider`].
///
/// ```
/// # egui::__run_test_ui(|ui| {
/// # let mut my_f32: f32 = 0.0;
/// ui.add(egui::DragValue::new(&mut my_f32).speed(0.1));
/// # });
/// ```
#[must_use = "You should put this widget in a ui with `ui.add(widget);`"]
pub struct DragValue<'a> {
    get_set_value: GetSetValue<'a>,
    speed: f64,
    prefix: String,
    suffix: String,
    range: RangeInclusive<f64>,
    clamp_existing_to_range: bool,
    min_decimals: usize,
    max_decimals: Option<usize>,
    custom_formatter: Option<NumFormatter<'a>>,
	delta_offset: f32,
}

impl<'a> DragValue<'a> {
    pub fn new<Num: emath::Numeric>(value: &'a mut Num) -> Self {
        let slf = Self::from_get_set(move |v: Option<f64>| {
            if let Some(v) = v {
                *value = Num::from_f64(v);
            }
            value.to_f64()
        });

        if Num::INTEGRAL {
            slf.max_decimals(0).range(Num::MIN..=Num::MAX).speed(0.25)
        } else {
            slf
        }
    }

    pub fn from_get_set(get_set_value: impl 'a + FnMut(Option<f64>) -> f64) -> Self {
        Self {
            get_set_value: Box::new(get_set_value),
            speed: 1.0,
            prefix: Default::default(),
            suffix: Default::default(),
            range: f64::NEG_INFINITY..=f64::INFINITY,
            clamp_existing_to_range: true,
            min_decimals: 0,
            max_decimals: None,
            custom_formatter: None,
			delta_offset: 0.0
        }
    }

    /// How much the value changes when dragged one point (logical pixel).
    ///
    /// Should be finite and greater than zero.
    #[inline]
    pub fn speed(mut self, speed: impl Into<f64>) -> Self {
        self.speed = speed.into();
        self
    }

    /// Sets valid range for the value.
    ///
    /// By default all values are clamped to this range, even when not interacted with.
    /// You can change this behavior by passing `false` to [`Self::clamp_existing_to_range`].
    #[deprecated = "Use `range` instead"]
    #[inline]
    pub fn clamp_range<Num: emath::Numeric>(self, range: RangeInclusive<Num>) -> Self {
        self.range(range)
    }

    /// Sets valid range for dragging the value.
    ///
    /// By default all values are clamped to this range, even when not interacted with.
    /// You can change this behavior by passing `false` to [`Self::clamp_existing_to_range`].
    #[inline]
    pub fn range<Num: emath::Numeric>(mut self, range: RangeInclusive<Num>) -> Self {
        self.range = range.start().to_f64()..=range.end().to_f64();
        self
    }

    /// If set to `true`, existing values will be clamped to [`Self::range`].
    ///
    /// If `false`, only values entered by the user (via dragging or text editing)
    /// will be clamped to the range.
    ///
    /// ### Without calling `range`
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// let mut my_value: f32 = 1337.0;
    /// ui.add(egui::DragValue::new(&mut my_value));
    /// assert_eq!(my_value, 1337.0, "No range, no clamp");
    /// # });
    /// ```
    ///
    /// ### With `.clamp_existing_to_range(true)` (default)
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// let mut my_value: f32 = 1337.0;
    /// ui.add(egui::DragValue::new(&mut my_value).range(0.0..=1.0));
    /// assert!(0.0 <= my_value && my_value <= 1.0, "Existing values should be clamped");
    /// # });
    /// ```
    ///
    /// ### With `.clamp_existing_to_range(false)`
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// let mut my_value: f32 = 1337.0;
    /// let response = ui.add(
    ///     egui::DragValue::new(&mut my_value).range(0.0..=1.0)
    ///         .clamp_existing_to_range(false)
    /// );
    /// if response.dragged() {
    ///     // The user edited the value, so it should be clamped to the range
    ///     assert!(0.0 <= my_value && my_value <= 1.0);
    /// } else {
    ///     // The user didn't edit, so our original value should still be here:
    ///     assert_eq!(my_value, 1337.0);
    /// }
    /// # });
    /// ```
    #[inline]
    pub fn clamp_existing_to_range(mut self, clamp_existing_to_range: bool) -> Self {
        self.clamp_existing_to_range = clamp_existing_to_range;
        self
    }

    #[inline]
    #[deprecated = "Renamed clamp_existing_to_range"]
    pub fn clamp_to_range(self, clamp_to_range: bool) -> Self {
        self.clamp_existing_to_range(clamp_to_range)
    }

    /// Show a prefix before the number, e.g. "x: "
    #[inline]
    pub fn prefix(mut self, prefix: impl ToString) -> Self {
        self.prefix = prefix.to_string();
        self
    }

    /// Add a suffix to the number, this can be e.g. a unit ("°" or " m")
    #[inline]
    pub fn suffix(mut self, suffix: impl ToString) -> Self {
        self.suffix = suffix.to_string();
        self
    }

    // TODO(emilk): we should also have a "min precision".
    /// Set a minimum number of decimals to display.
    /// Normally you don't need to pick a precision, as the slider will intelligently pick a precision for you.
    /// Regardless of precision the slider will use "smart aim" to help the user select nice, round values.
    #[inline]
    pub fn min_decimals(mut self, min_decimals: usize) -> Self {
        self.min_decimals = min_decimals;
        self
    }

    // TODO(emilk): we should also have a "max precision".
    /// Set a maximum number of decimals to display.
    /// Values will also be rounded to this number of decimals.
    /// Normally you don't need to pick a precision, as the slider will intelligently pick a precision for you.
    /// Regardless of precision the slider will use "smart aim" to help the user select nice, round values.
    #[inline]
    pub fn max_decimals(mut self, max_decimals: usize) -> Self {
        self.max_decimals = Some(max_decimals);
        self
    }

    #[inline]
    pub fn max_decimals_opt(mut self, max_decimals: Option<usize>) -> Self {
        self.max_decimals = max_decimals;
        self
    }

    /// Set an exact number of decimals to display.
    /// Values will also be rounded to this number of decimals.
    /// Normally you don't need to pick a precision, as the slider will intelligently pick a precision for you.
    /// Regardless of precision the slider will use "smart aim" to help the user select nice, round values.
    #[inline]
    pub fn fixed_decimals(mut self, num_decimals: usize) -> Self {
        self.min_decimals = num_decimals;
        self.max_decimals = Some(num_decimals);
        self
    }

    /// Set custom formatter defining how numbers are converted into text.
    ///
    /// A custom formatter takes a `f64` for the numeric value and a `RangeInclusive<usize>` representing
    /// the decimal range i.e. minimum and maximum number of decimal places shown.
    ///
    /// The default formatter is [`crate::Style::number_formatter`].
    ///
    /// See also: [`DragValue::custom_parser`]
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let mut my_i32: i32 = 0;
    /// ui.add(egui::DragValue::new(&mut my_i32)
    ///     .range(0..=((60 * 60 * 24) - 1))
    ///     .custom_formatter(|n, _| {
    ///         let n = n as i32;
    ///         let hours = n / (60 * 60);
    ///         let mins = (n / 60) % 60;
    ///         let secs = n % 60;
    ///         format!("{hours:02}:{mins:02}:{secs:02}")
    ///     })
    ///     .custom_parser(|s| {
    ///         let parts: Vec<&str> = s.split(':').collect();
    ///         if parts.len() == 3 {
    ///             parts[0].parse::<i32>().and_then(|h| {
    ///                 parts[1].parse::<i32>().and_then(|m| {
    ///                     parts[2].parse::<i32>().map(|s| {
    ///                         ((h * 60 * 60) + (m * 60) + s) as f64
    ///                     })
    ///                 })
    ///             })
    ///             .ok()
    ///         } else {
    ///             None
    ///         }
    ///     }));
    /// # });
    /// ```
    pub fn custom_formatter(
        mut self,
        formatter: impl 'a + Fn(f64, RangeInclusive<usize>) -> String,
    ) -> Self {
        self.custom_formatter = Some(Box::new(formatter));
        self
    }

    /// Set `custom_formatter` and `custom_parser` to display and parse numbers as binary integers. Floating point
    /// numbers are *not* supported.
    ///
    /// `min_width` specifies the minimum number of displayed digits; if the number is shorter than this, it will be
    /// prefixed with additional 0s to match `min_width`.
    ///
    /// If `twos_complement` is true, negative values will be displayed as the 2's complement representation. Otherwise
    /// they will be prefixed with a '-' sign.
    ///
    /// # Panics
    ///
    /// Panics if `min_width` is 0.
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let mut my_i32: i32 = 0;
    /// ui.add(egui::DragValue::new(&mut my_i32).binary(64, false));
    /// # });
    /// ```
    pub fn binary(self, min_width: usize, twos_complement: bool) -> Self {
        assert!(
            min_width > 0,
            "DragValue::binary: `min_width` must be greater than 0"
        );
        if twos_complement {
            self.custom_formatter(move |n, _| format!("{:0>min_width$b}", n as i64))
        } else {
            self.custom_formatter(move |n, _| {
                let sign = if n < 0.0 { MINUS_CHAR_STR } else { "" };
                format!("{sign}{:0>min_width$b}", n.abs() as i64)
            })
        }
    }

    /// Set `custom_formatter` and `custom_parser` to display and parse numbers as octal integers. Floating point
    /// numbers are *not* supported.
    ///
    /// `min_width` specifies the minimum number of displayed digits; if the number is shorter than this, it will be
    /// prefixed with additional 0s to match `min_width`.
    ///
    /// If `twos_complement` is true, negative values will be displayed as the 2's complement representation. Otherwise
    /// they will be prefixed with a '-' sign.
    ///
    /// # Panics
    ///
    /// Panics if `min_width` is 0.
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let mut my_i32: i32 = 0;
    /// ui.add(egui::DragValue::new(&mut my_i32).octal(22, false));
    /// # });
    /// ```
    pub fn octal(self, min_width: usize, twos_complement: bool) -> Self {
        assert!(
            min_width > 0,
            "DragValue::octal: `min_width` must be greater than 0"
        );
        if twos_complement {
            self.custom_formatter(move |n, _| format!("{:0>min_width$o}", n as i64))
        } else {
            self.custom_formatter(move |n, _| {
                let sign = if n < 0.0 { MINUS_CHAR_STR } else { "" };
                format!("{sign}{:0>min_width$o}", n.abs() as i64)
            })
        }
    }

    /// Set `custom_formatter` and `custom_parser` to display and parse numbers as hexadecimal integers. Floating point
    /// numbers are *not* supported.
    ///
    /// `min_width` specifies the minimum number of displayed digits; if the number is shorter than this, it will be
    /// prefixed with additional 0s to match `min_width`.
    ///
    /// If `twos_complement` is true, negative values will be displayed as the 2's complement representation. Otherwise
    /// they will be prefixed with a '-' sign.
    ///
    /// # Panics
    ///
    /// Panics if `min_width` is 0.
    ///
    /// ```
    /// # egui::__run_test_ui(|ui| {
    /// # let mut my_i32: i32 = 0;
    /// ui.add(egui::DragValue::new(&mut my_i32).hexadecimal(16, false, true));
    /// # });
    /// ```
    pub fn hexadecimal(self, min_width: usize, twos_complement: bool, upper: bool) -> Self {
        assert!(
            min_width > 0,
            "DragValue::hexadecimal: `min_width` must be greater than 0"
        );
        match (twos_complement, upper) {
            (true, true) => {
                self.custom_formatter(move |n, _| format!("{:0>min_width$X}", n as i64))
            }
            (true, false) => {
                self.custom_formatter(move |n, _| format!("{:0>min_width$x}", n as i64))
            }
            (false, true) => self.custom_formatter(move |n, _| {
                let sign = if n < 0.0 { MINUS_CHAR_STR } else { "" };
                format!("{sign}{:0>min_width$X}", n.abs() as i64)
            }),
            (false, false) => self.custom_formatter(move |n, _| {
                let sign = if n < 0.0 { MINUS_CHAR_STR } else { "" };
                format!("{sign}{:0>min_width$x}", n.abs() as i64)
            }),
        }
    }
	

    pub fn with_delta_offset(mut self, delta_offset: f32) -> Self {
        self.delta_offset = delta_offset;
        self
    }

}

impl Widget for DragValue<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let Self {
            mut get_set_value,
            speed,
            range,
            clamp_existing_to_range,
            prefix,
            suffix,
            min_decimals,
            max_decimals,
            custom_formatter,
			delta_offset
        } = self;

        let shift = ui.input(|i| i.modifiers.command);
        // The widget has the same ID whether it's in edit or button mode.
        let id = ui.next_auto_id();
        let is_slow_speed = shift && ui.ctx().is_being_dragged(id);


        if ui.memory_mut(|mem| !mem.had_focus_last_frame(id) && mem.has_focus(id)) {
            ui.data_mut(|data| data.remove::<String>(id));
        }

        let old_value = get(&mut get_set_value);
        let mut value = old_value;
        let aim_rad = ui.input(|i| i.aim_radius() as f64);

        let auto_decimals = (aim_rad / speed.abs()).log10().ceil().clamp(0.0, 15.0) as usize;
        let auto_decimals = auto_decimals + is_slow_speed as usize;
        let max_decimals = max_decimals
            .unwrap_or(auto_decimals + 2)
            .at_least(min_decimals);
        let auto_decimals = auto_decimals.clamp(min_decimals, max_decimals);

        if clamp_existing_to_range {
            value = clamp_value_to_range(value, range.clone());
        }

        if old_value != value {
            set(&mut get_set_value, value);
            ui.data_mut(|data| data.remove::<String>(id));
        }

        let value_text = match custom_formatter {
            Some(custom_formatter) => custom_formatter(value, auto_decimals..=max_decimals),
            None => ui
                .style()
                .number_formatter
                .format(value, auto_decimals..=max_decimals),
        };

        let text_style = ui.style().drag_value_text_style.clone();

        // some clones below are redundant if AccessKit is disabled
        #[allow(clippy::redundant_clone)]
        let mut response = {
            let left_down = ui.input(|state| state.pointer.primary_down());
            let sense = if left_down { Sense::drag() } else { Sense::click_and_drag() };
            let button = Button::new(
                RichText::new(format!("{}{}{}", prefix, value_text.clone(), suffix))
                    .text_style(text_style),
            )
            .wrap_mode(TextWrapMode::Extend)
            .sense(sense)
            .min_size(ui.spacing().interact_size); // TODO(emilk): find some more generic solution to `min_size`

            let response = ui.add(button);

            if ui.input(|i| i.pointer.primary_pressed() || i.pointer.primary_released()) {
                // Reset memory of preciely dagged value.
                ui.data_mut(|data| data.remove::<f64>(id));
            }

            if response.dragged_by(egui::PointerButton::Primary) {
                let mdelta = response.drag_delta() + egui::Vec2::new(0.0, delta_offset);
                let delta_points = -mdelta.y; // Increase up

                let speed = if is_slow_speed { speed / 10.0 } else { speed };

                let delta_value = delta_points as f64 * speed;

                if delta_value != 0.0 {
                    // Since we round the value being dragged, we need to store the full precision value in memory:
                    let precise_value = ui.data_mut(|data| data.get_temp::<f64>(id));
                    let precise_value = precise_value.unwrap_or(value);
                    let precise_value = precise_value + delta_value;

                    let aim_delta = aim_rad * speed;
                    let rounded_new_value = emath::smart_aim::best_in_range_f64(
                        precise_value - aim_delta,
                        precise_value + aim_delta,
                    );
                    let rounded_new_value =
                        emath::round_to_decimals(rounded_new_value, auto_decimals);
                    // Dragging will always clamp the value to the range.
                    let rounded_new_value = clamp_value_to_range(rounded_new_value, range.clone());
                    set(&mut get_set_value, rounded_new_value);

                    ui.data_mut(|data| data.insert_temp::<f64>(id, precise_value));
                }
            }

            response
        };

        if get(&mut get_set_value) != old_value {
            response.mark_changed();
        }

        response.widget_info(|| WidgetInfo::drag_value(ui.is_enabled(), value));
        response
    }
}

fn parse(custom_parser: &Option<NumParser<'_>>, value_text: &str) -> Option<f64> {
    match &custom_parser {
        Some(parser) => parser(value_text),
        None => default_parser(value_text),
    }
}

/// The default egui parser of numbers.
///
/// It ignored whitespaces anywhere in the input, and treats the special minus character (U+2212) as a normal minus.
fn default_parser(text: &str) -> Option<f64> {
    let text: String = text
        .chars()
        // Ignore whitespace (trailing, leading, and thousands separators):
        .filter(|c| !c.is_whitespace())
        // Replace special minus character with normal minus (hyphen):
        .map(|c| if c == '−' { '-' } else { c })
        .collect();

    text.parse().ok()
}

/// Clamp the given value with careful handling of negative zero, and other corner cases.
pub(crate) fn clamp_value_to_range(x: f64, range: RangeInclusive<f64>) -> f64 {
    let (mut min, mut max) = (*range.start(), *range.end());

    if min.total_cmp(&max) == Ordering::Greater {
        (min, max) = (max, min);
    }

    match x.total_cmp(&min) {
        Ordering::Less | Ordering::Equal => min,
        Ordering::Greater => match x.total_cmp(&max) {
            Ordering::Greater | Ordering::Equal => max,
            Ordering::Less => x,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::clamp_value_to_range;

    macro_rules! total_assert_eq {
        ($a:expr, $b:expr) => {
            assert!(
                matches!($a.total_cmp(&$b), std::cmp::Ordering::Equal),
                "{} != {}",
                $a,
                $b
            );
        };
    }

    #[test]
    fn test_total_cmp_clamp_value_to_range() {
        total_assert_eq!(0.0_f64, clamp_value_to_range(-0.0, 0.0..=f64::MAX));
        total_assert_eq!(-0.0_f64, clamp_value_to_range(0.0, -1.0..=-0.0));
        total_assert_eq!(-1.0_f64, clamp_value_to_range(-25.0, -1.0..=1.0));
        total_assert_eq!(5.0_f64, clamp_value_to_range(5.0, -1.0..=10.0));
        total_assert_eq!(15.0_f64, clamp_value_to_range(25.0, -1.0..=15.0));
        total_assert_eq!(1.0_f64, clamp_value_to_range(1.0, 1.0..=10.0));
        total_assert_eq!(10.0_f64, clamp_value_to_range(10.0, 1.0..=10.0));
        total_assert_eq!(5.0_f64, clamp_value_to_range(5.0, 10.0..=1.0));
        total_assert_eq!(5.0_f64, clamp_value_to_range(15.0, 5.0..=1.0));
        total_assert_eq!(1.0_f64, clamp_value_to_range(-5.0, 5.0..=1.0));
    }

    #[test]
    fn test_default_parser() {
        assert_eq!(super::default_parser("123"), Some(123.0));

        assert_eq!(super::default_parser("1.23"), Some(1.230));

        assert_eq!(
            super::default_parser(" 1.23 "),
            Some(1.230),
            "We should handle leading and trailing spaces"
        );

        assert_eq!(
            super::default_parser("1 234 567"),
            Some(1_234_567.0),
            "We should handle thousands separators using half-space"
        );

        assert_eq!(
            super::default_parser("-1.23"),
            Some(-1.23),
            "Should handle normal hyphen as minus character"
        );
        assert_eq!(
            super::default_parser("−1.23"),
            Some(-1.23),
            "Should handle special minus character (https://www.compart.com/en/unicode/U+2212)"
        );
    }
}
