//! 设置: preferences (`preferences.json` in the library root) and the
//! settings view — a category list in the sidebar, one page per category,
//! each page a column of grouped rows. Only what a
//! user really needs to change lives here.

use crate::{app::{View, Workspace}, theme::{self, craft, Theme}, themes, ui};
use crate::ui::{TitleBarExt, TipExt};
use crate::sidebar as sb;
use gpui::{prelude::FluentBuilder, *};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

// ---- preferences

static FEISHU: AtomicBool = AtomicBool::new(false);
static ENGLISH: AtomicBool = AtomicBool::new(false);
static PREFS: RwLock<Option<Preferences>> = RwLock::new(None);

/// The active document theme draws blocks the 飞书 way (else Craft's).
pub fn feishu() -> bool { FEISHU.load(Ordering::Relaxed) }
pub(crate) fn set_base(feishu: bool) { FEISHU.store(feishu, Ordering::Relaxed); }
pub fn english() -> bool { ENGLISH.load(Ordering::Relaxed) }
pub fn tr(zh: &'static str, en: &'static str) -> &'static str { if english() { en } else { zh } }

#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    /// Document theme id (themes.rs); empty in files from before themes, see `feishu`.
    pub theme: String,
    /// The old two-way switch, still written so older builds keep the base style.
    pub feishu: bool,
    pub english: bool,
    /// Fetch `http(s)` images referenced by documents.
    pub remote_images: bool,
    /// The system-wide quick note hotkey is registered.
    pub quick_hotkey: bool,
}

impl Default for Preferences {
    fn default() -> Self { Preferences { theme: String::new(), feishu: false, english: false, remote_images: true, quick_hotkey: true } }
}

impl Preferences {
    pub fn theme_id(&self) -> String {
        if !self.theme.is_empty() { self.theme.clone() } else if self.feishu { "feishu".into() } else { "craft".into() }
    }
}

fn path() -> std::path::PathBuf { crate::store::Store::root_dir().join("preferences.json") }

pub fn load() {
    let p = std::fs::read(path()).ok().and_then(|b| serde_json::from_slice::<Preferences>(&b).ok()).unwrap_or_default();
    apply(p);
}

fn apply(p: Preferences) {
    FEISHU.store(p.feishu, Ordering::Relaxed);
    ENGLISH.store(p.english, Ordering::Relaxed);
    // Before the themes are read (startup) the base comes from `feishu`.
    themes::select(&p.theme_id());
    *PREFS.write().unwrap() = Some(p);
}

/// The current preferences (loaded on first use).
pub fn prefs() -> Preferences {
    if PREFS.read().unwrap().is_none() { load(); }
    PREFS.read().unwrap().clone().unwrap_or_default()
}

fn save(p: &Preferences) -> anyhow::Result<()> {
    use std::io::Write;
    let root = crate::store::Store::root_dir();
    std::fs::create_dir_all(&root)?;
    let tmp = root.join("preferences.json.tmp");
    let mut f = std::fs::File::create(&tmp)?;
    f.write_all(&serde_json::to_vec_pretty(p)?)?;
    f.sync_all()?;
    std::fs::rename(tmp, path())?;
    apply(p.clone());
    Ok(())
}

/// Redraw every window's document after the theme or language changed.
pub(crate) fn refresh_all(this: &mut Workspace, cx: &mut Context<Workspace>) {
    fn refresh(ws: &mut Workspace, cx: &mut Context<Workspace>) {
        ws.settings_filter.update(cx, |f, cx| f.set_placeholder(tr("搜索快捷键", "Search shortcuts"), cx));
        if let Some(editor) = &ws.editor {
            editor.update(cx, |e, cx| { e.cache.borrow_mut().clear(); e.refresh_theme(); cx.notify(); });
        }
        cx.notify();
    }
    refresh(this, cx);
    // The other windows. Skip this workspace, and never read a window that is
    // mid-update (the one a click came from): GPUI panics on that, so ask
    // `is_active` first, which is None for a window on the update stack.
    let me = cx.entity_id();
    for w in cx.windows() {
        if let Some(h) = w.downcast::<Workspace>()
            && h.is_active(cx).is_some()
            && h.entity(cx).is_ok_and(|e| e.entity_id() != me)
        {
            let _ = h.update(cx, |ws, _, cx| refresh(ws, cx));
        }
    }
    crate::app::set_menus(cx);
    cx.refresh_windows();
}

/// A theme file in the user's editor; scripted runs only log it (they must
/// not start other apps).
fn open_in_editor(p: &std::path::Path, cx: &mut App) {
    if std::env::var_os("FDL_SCRIPT").is_some() { eprintln!("devtools: open {}", p.display()); return }
    cx.open_url(&format!("file://{}", p.display()));
}

/// Theme files are watched: saving one in an editor redraws at once.
pub(crate) fn watch_themes(cx: &mut Context<Workspace>) {
    static LAST: Mutex<Option<Vec<(std::path::PathBuf, std::time::SystemTime, u64)>>> = Mutex::new(None);
    *LAST.lock().unwrap() = Some(themes::stamp());
    cx.spawn(async |this, cx| loop {
        cx.background_executor().timer(std::time::Duration::from_millis(800)).await;
        let now = cx.background_executor().spawn(async { themes::stamp() }).await;
        let changed = { let mut l = LAST.lock().unwrap(); let c = l.as_ref() != Some(&now); *l = Some(now); c };
        if changed && this.update(cx, |w, cx| w.reload_themes(cx)).is_err() { break }
        if this.upgrade().is_none() { break }
    }).detach();
}

// ---- the view

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page { #[default] General, Appearance, Shortcuts, Library, About }

impl Page {
    fn title(self) -> &'static str {
        match self {
            Page::General => tr("通用", "General"),
            Page::Appearance => tr("外观", "Appearance"),
            Page::Shortcuts => tr("快捷键", "Shortcuts"),
            Page::Library => tr("文档库", "Library"),
            Page::About => tr("关于", "About"),
        }
    }
    fn icon(self) -> &'static str {
        match self {
            Page::General => "view-settings",
            Page::Appearance => "palette",
            Page::Shortcuts => "keyboard",
            Page::Library => "storage",
            Page::About => "info",
        }
    }
}

/// The sidebar list, one evenly spaced run (six rows need no unlabelled
/// groups); `None` is 回收站 (its own view, shown with this sidebar).
const NAV: [Option<Page>; 6] = [Some(Page::General), Some(Page::Appearance), Some(Page::Shortcuts), Some(Page::Library), None, Some(Page::About)];

// ---- controls (switch on the accent, 30pt buttons, grouped rows)

const COLUMN_W: f32 = 640.;

/// A titled group: the caption above, then the rows in one rounded card,
/// split by hairlines.
pub(crate) fn group(title: Option<&'static str>, rows: Vec<AnyElement>) -> Div {
    let n = rows.len();
    div().w_full().flex().flex_col().gap(px(8.))
        .children(title.map(caption))
        .child(div().w_full().flex().flex_col().rounded(px(12.)).border_1().border_color(Theme::line_border()).bg(Theme::bg_body()).overflow_hidden()
            .children(rows.into_iter().enumerate().map(move |(i, r)| div().w_full().flex().flex_col().when(i + 1 < n, |d| d.border_b_1().border_color(ui::list_line())).child(r))))
}

pub(crate) fn caption(t: &'static str) -> Div {
    div().px(px(4.)).text_size(px(13.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::sidebar_section()).child(t)
}

/// One setting: title and an optional description on the left, the control on the right.
fn row(title: impl Into<SharedString>, desc: Option<SharedString>, control: impl IntoElement) -> Div {
    div().w_full().min_h(px(48.)).px(px(16.)).py(px(10.)).flex().items_center().justify_between().gap(px(24.))
        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.))
            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text()).child(title.into()))
            .children(desc.map(|d| div().text_size(px(12.)).line_height(px(17.)).text_color(Theme::text_caption()).child(d))))
        .child(div().flex_none().flex().items_center().gap(px(8.)).child(control))
}

fn desc(s: &'static str) -> Option<SharedString> { Some(s.into()) }

/// The switch: the accent when on, black 16% when off, a white knob.
pub(crate) fn switch(id: impl Into<ElementId>, on: bool) -> Stateful<Div> {
    div().id(id).flex_none().w(px(32.)).h(px(19.)).p(px(2.)).rounded_full().cursor_pointer().flex().items_center()
        .when(on, |d| d.bg(Theme::primary()).justify_end())
        .when(!on, |d| d.bg(theme::ca(craft::N1000, 0.16)).justify_start())
        .child(div().size(px(15.)).rounded_full().bg(gpui::white()).shadow(theme::shadow_button()))
}

/// A segmented control: a grey groove and a white selected pill.
fn segmented(id: &'static str, labels: &[&'static str], selected: usize, on: impl Fn(&mut Workspace, usize, &mut Context<Workspace>) + Clone + 'static, cx: &mut Context<Workspace>) -> Div {
    div().flex_none().h(px(28.)).p(px(2.)).flex().gap(px(2.)).rounded(px(9.)).bg(Theme::fill_hover_strong())
        .children(labels.iter().enumerate().map(|(i, label)| {
            let on = on.clone();
            div().id((id, i)).h_full().px(px(12.)).flex().items_center().justify_center().rounded(px(7.)).cursor_pointer()
                .text_size(px(13.)).line_height(px(18.))
                .when(i == selected, |d| d.bg(Theme::bg_body()).shadow(theme::shadow_button()).text_color(Theme::text()))
                .when(i != selected, |d| d.text_color(Theme::text_caption()).hover(|s| s.text_color(Theme::text())))
                .child(*label)
                .on_click(cx.listener(move |w, _, _, cx| on(w, i, cx)))
        }))
}

/// A keyboard shortcut chip; `recording` shows it waiting for keys.
pub(crate) fn keycap(id: impl Into<ElementId>, key: &str, recording: bool) -> Stateful<Div> {
    div().id(id).flex_none().min_w(px(144.)).min_h(px(28.)).px(px(14.)).py(px(4.)).flex().items_center().justify_center().rounded(px(7.)).cursor_pointer()
        .text_size(px(12.)).line_height(px(16.))
        .when(recording, |d| d.bg(Theme::bg_body()).border_1().border_color(Theme::primary()).text_color(Theme::primary()).child(if key.is_empty() { tr("按下新的组合键…", "Press keys…").to_string() } else { crate::shortcuts::pretty(key) }))
        .when(!recording, |d| d.bg(Theme::fill_hover()).border_1().border_color(transparent_black()).text_color(Theme::text()).hover(|s| s.bg(Theme::fill_hover_strong())).child(crate::shortcuts::pretty(key)))
}

/// A small text button (恢复默认 …).
pub(crate) fn link_button(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div().id(id).flex_none().h(px(26.)).px(px(10.)).flex().items_center().rounded(px(7.)).cursor_pointer()
        .text_size(px(12.)).line_height(px(16.)).text_color(Theme::primary()).hover(|s| s.bg(Theme::fill_hover()))
        .child(label)
}

pub(crate) fn button(id: impl Into<ElementId>, label: &'static str) -> Stateful<Div> {
    div().id(id).flex_none().h(px(28.)).px(px(12.)).flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
        .bg(Theme::bg_body()).border_1().border_color(Theme::button_border()).shadow(theme::shadow_button())
        .hover(|s| s.bg(theme::c(craft::N100))).active(|s| s.bg(theme::c(craft::N200)))
        .text_size(px(13.)).line_height(px(18.)).text_color(Theme::text()).child(label)
}

// ---- theme previews: a small page drawn with the theme's own values

const CARD_W: f32 = 312.;
const CARD_H: f32 = 206.;
/// Preview scale against the real document: the dropdown's small one, the page's larger one.
/// Its frame clips to a rectangle (GPUI masks have no radius), so the page is
/// rounded itself: 7 inside the frame's 8 and 1pt border.
const S: f32 = 0.62;
const PAGE_S: f32 = 0.78;

fn preview(t: &themes::Theme, s: f32) -> Div {
    let st = t.style;
    let sc = |v: f32| px((v * s).max(0.5));
    let text = st.color("page.color").unwrap_or_else(|| st.token("text"));
    let (body, line) = (st.num("page.size"), st.num("page.line"));
    let (font, mono): (SharedString, SharedString) = (st.font("page.font").into(), st.font("pre.font").into());
    let link = st.color("a.color").unwrap_or(text);
    let small = |s: &'static str| div().text_size(sc(body)).line_height(sc(line)).child(s);
    let para = div().flex().flex_wrap().text_size(sc(body)).line_height(sc(line))
        .child(tr("好的设计会让人", "Good design is "))
        .child(div().font_weight(FontWeight(st.num("strong.weight"))).text_color(st.color("strong.color").unwrap_or(text)).child(tr("忘记它的存在", "invisible")))
        .child(tr("，", ", "))
        .child(div().text_color(link).when(st.flag("a.underline"), |d| d.underline()).child(tr("读原文", "read on")))
        .child(div().ml(px(4.)).px(sc(st.num("code.pad")).max(px(2.))).rounded(sc(st.num("code.radius"))).bg(st.token("code_chip"))
            .border_1().border_color(st.token("code_chip_border")).font_family(mono.clone()).text_color(st.color("code.color").unwrap_or(text)).child("css"));
    let bullet_c = st.color("bullet.color").unwrap_or(text);
    let bullet = |s: &'static str| div().flex().items_center().gap(px(6.)).child(div().size(sc(5.).max(px(3.))).rounded_full().bg(bullet_c)).child(small(s));
    let task_size = st.num("task.size");
    let task = div().flex().items_center().gap(px(5.))
        .child(div().size(sc(task_size)).rounded(sc(st.num("task.radius"))).bg(st.color("task.checked").unwrap_or_else(|| st.token("primary"))).flex().items_center().justify_center()
            .child(ui::icon("todo-check", (task_size * s * 0.75).round(), st.color("task.check").unwrap_or_else(gpui::white))))
        .child(small(tr("整理读书卡片", "Sort the cards")).text_color(st.token("todo_done")).when(st.flag("done.strike"), |d| d.line_through()));
    let quote = div().flex().gap(sc((st.num("quote.indent") - st.num("quote.bar")).max(4.)))
        .child(div().w(sc(st.num("quote.bar")).max(px(1.))).rounded(px(1.)).bg(st.token("quote_bar")))
        .child(small(tr("少即是多。", "Less is more.")).text_color(st.color("quote.color").unwrap_or(text)).when(st.flag("quote.italic"), |d| d.italic()));
    let (hr_c, hr_w) = (st.token("separator_dots"), (st.num("hr.width") * s).max(0.6));
    let hr = match st.line("hr.style") {
        themes::LineStyle::Solid => div().w_full().h(px(hr_w)).bg(hr_c),
        themes::LineStyle::Dashed => div().w_full().flex().gap(px(4.)).overflow_hidden().children((0..40).map(move |_| div().flex_none().w(px(4.)).h(px(hr_w)).bg(hr_c))),
        themes::LineStyle::Dotted => div().w_full().flex().gap(px(2.5)).overflow_hidden().children((0..90).map(move |_| div().flex_none().size(px(hr_w)).rounded_full().bg(hr_c))),
    };
    let code = div().w_full().px(px(8.)).py(px(5.)).rounded(sc(st.num("pre.radius"))).bg(st.token("code_bg")).border_1().border_color(st.token("code_border"))
        .font_family(mono).text_size(sc(st.num("pre.size"))).line_height(sc(st.num("pre.line"))).text_color(st.token("code_text"))
        .child("let theme = \"css\";");
    // Block spacing keeps its proportions up to a point, so every theme fits the card.
    div().size_full().px(px(18.)).pt(px(14.)).flex().flex_col().gap(sc(st.num("p.gap")).clamp(px(3.), px(7.))).bg(st.color("page.bg").unwrap_or_else(|| st.token("bg_body")))
        .font_family(font).text_color(text).overflow_hidden()
        .child(div().text_size(sc(st.num("title.size"))).line_height(sc(st.num("title.size") * 1.25)).font_weight(FontWeight(st.num("title.weight"))).text_color(st.color("title.color").unwrap_or(text)).child(tr("读书笔记", "Reading notes")))
        .child(div().text_size(sc(st.num("h2.size"))).line_height(sc(st.num("h2.line"))).font_weight(FontWeight(st.num("h2.weight"))).text_color(st.color("h2.color").unwrap_or(text)).child(tr("摘录", "Highlights")))
        .child(para)
        .child(div().flex().gap(px(18.)).child(bullet(tr("先观察", "Look first"))).child(bullet(tr("再动手", "Then build"))))
        .child(task)
        .child(quote)
        .child(div().py(px(2.)).child(hr))
        .child(code)
}

/// 外观's sample page: a short document with most of what a theme styles
/// (headings, inline styles, lists, a task, quote, callout, table, divider,
/// highlighted code), at scale `s`, in two columns so it reads at a glance.
fn sample_page(t: &themes::Theme, s: f32) -> Div {
    let st = t.style;
    let sc = |v: f32| px((v * s).max(0.5));
    let text = st.color("page.color").unwrap_or_else(|| st.token("text"));
    let (body, line) = (st.num("page.size"), st.num("page.line"));
    let (font, mono): (SharedString, SharedString) = (st.font("page.font").into(), st.font("pre.font").into());
    let gap = sc(st.num("p.gap")).clamp(px(4.), px(9.));
    let p = |c: Div| c.text_size(sc(body)).line_height(sc(line));
    let heading = |l: u8, s: &'static str| {
        let h = format!("h{l}");
        div().text_size(sc(st.num(&format!("{h}.size")))).line_height(sc(st.num(&format!("{h}.line")))).font_weight(FontWeight(st.num(&format!("{h}.weight"))))
            .text_color(st.color(&format!("{h}.color")).unwrap_or(text)).child(s)
    };
    let link = st.color("a.color").unwrap_or(text);
    let chip = |s: &'static str| div().px(sc(st.num("code.pad")).max(px(2.))).rounded(sc(st.num("code.radius"))).bg(st.color("code.bg").unwrap_or_else(|| st.token("code_chip")))
        .border_1().border_color(st.color("code.border").unwrap_or_else(|| st.token("code_chip_border"))).font_family(mono.clone()).text_color(st.color("code.color").unwrap_or(text)).child(s);
    let para = p(div().flex().flex_wrap().items_center())
        .child(tr("quiet.md 是一款", "quiet.md is a "))
        .child(div().font_weight(FontWeight(st.num("strong.weight"))).text_color(st.color("strong.color").unwrap_or(text)).child(tr("安静的本地笔记应用", "quiet, local notes app")))
        .child(tr("，", ", "))
        .child(tr("文档就是电脑上的文件。", "your notes are files. "))
        .child(tr("输入 ", "Type "))
        .child(chip("/"))
        .child(tr(" 插入内容，", " to insert, "))
        .child(tr("详见", "see "))
        // The full stop rides with the link, so it never wraps alone.
        .child(div().flex().child(div().text_color(link).when(st.flag("a.underline"), |d| d.underline()).child(tr("写作基础", "Writing basics"))).child(tr("。", ".")));
    let indent = sc(st.num("list.indent")).max(px(10.));
    let marker = |m: Div| div().flex_none().w(indent).flex().justify_center().child(m);
    let bullet_c = st.color("bullet.color").unwrap_or(text);
    let bullet = |s: &'static str| div().flex().items_center().child(marker(div().size(sc(6.).max(px(3.))).rounded_full().bg(bullet_c))).child(p(div()).child(s));
    let num_c = st.color("number.color").unwrap_or(text);
    let number = |n: &'static str, s: &'static str| div().flex().items_center().child(marker(p(div()).text_color(num_c).child(n))).child(p(div()).child(s));
    let task_size = st.num("task.size");
    let task = |done: bool, label: &'static str| div().flex().items_center().child(marker(
            div().size(sc(task_size)).rounded(sc(st.num("task.radius"))).flex().items_center().justify_center()
                .when(done, |d| d.bg(st.color("task.checked").unwrap_or_else(|| st.token("primary")))
                    .child(ui::icon("todo-check", (task_size * s * 0.75).round(), st.color("task.check").unwrap_or_else(gpui::white))))
                .when(!done, |d| d.border_1().border_color(st.color("task.border").unwrap_or_else(|| st.token("todo_box"))))))
        .child(p(div()).child(label).when(done, |d| d.text_color(st.color("done.color").unwrap_or_else(|| st.token("todo_done"))).when(st.flag("done.strike"), |d| d.line_through())));
    let quote = div().flex().gap(sc((st.num("quote.indent") - st.num("quote.bar")).max(4.)))
        .child(div().flex_none().w(sc(st.num("quote.bar")).max(px(1.))).rounded(px(1.)).bg(st.color("quote.bar-color").unwrap_or_else(|| st.token("quote_bar"))))
        .child(p(div()).text_color(st.color("quote.color").unwrap_or(text)).when(st.flag("quote.italic"), |d| d.italic()).child(tr("每一篇指南都是用 quiet.md 写成的，放心修改。", "Every guide is written in quiet.md. Edit them freely.")));
    let callout = div().px(sc(st.num("callout.pad-x"))).py(sc(st.num("callout.pad-y"))).rounded(sc(st.num("callout.radius")))
        .bg(st.named("callout", "light-yellow").unwrap_or_else(|| st.token("code_chip")))
        .when(st.num("callout.border") > 0., |d| d.border(sc(st.num("callout.border")).max(px(1.))).border_color(st.named("border", "yellow").unwrap_or_else(|| st.token("code_chip_border"))))
        .child(p(div().flex().gap(px(6.))).child("💡").child(tr("一边读一边动手试，记得最快。", "Try things as you read; it sticks faster.")));
    let line_c = st.color("table.line").unwrap_or_else(|| st.token("table_line"));
    let cell = |s: &'static str, head: bool| div().flex_1().px(px(6.)).py(px(3.)).when(head, |d| d.bg(st.color("th.bg").unwrap_or_else(|| st.token("table_header"))).font_weight(FontWeight::SEMIBOLD)).child(s);
    let row = |a: &'static str, b: &'static str, head: bool, last: bool| div().flex().when(!last, |d| d.border_b_1().border_color(line_c))
        .child(cell(a, head).border_r_1().border_color(line_c)).child(cell(b, head));
    let table = div().w_full().rounded(sc(st.num("table.radius"))).border_1().border_color(line_c).overflow_hidden()
        .text_size(sc(st.num("table.size"))).line_height(sc(st.num("table.line-height")))
        .child(row(tr("按键", "Keys"), tr("作用", "Does"), true, false)).child(row("⌘ N", tr("新建文档", "New document"), false, false)).child(row("⌘ J", tr("搜索所有文档", "Search documents"), false, true));
    let (hr_c, hr_w) = (st.color("hr.color").unwrap_or_else(|| st.token("separator_dots")), (st.num("hr.width") * s).max(0.6));
    // Painted to the column's width: a row of fixed-size dots would set the
    // column's minimum width and squeeze its neighbour.
    let hr_style = st.line("hr.style");
    let hr = canvas(|_, _, _| {}, move |b, _, window, _| {
        let (step, dash) = match hr_style { themes::LineStyle::Solid => return window.paint_quad(fill(Bounds::new(b.origin, size(b.size.width, px(hr_w))), hr_c)), themes::LineStyle::Dashed => (8., 4.), themes::LineStyle::Dotted => (hr_w + 2.5, hr_w) };
        let mut x = 0.;
        while x + dash <= f32::from(b.size.width) {
            window.paint_quad(fill(Bounds::new(point(b.origin.x + px(x), b.origin.y), size(px(dash), px(hr_w))), hr_c).corner_radii(px(if hr_style == themes::LineStyle::Dotted { hr_w / 2. } else { 0. })));
            x += step;
        }
    }).w_full().h(px(hr_w));
    let syn = |k: &str| st.color(&format!("syntax.{k}")).unwrap_or(text);
    let tok = |s: &'static str, c: Hsla| div().text_color(c).child(s);
    let code_line = |n: &'static str, parts: Vec<Div>| div().flex()
        .when(st.flag("pre.numbers"), |d| d.child(div().flex_none().w(px(16.)).mr(px(8.)).text_color(st.color("pre.number-color").unwrap_or_else(|| st.token("code_line_number"))).child(n)))
        .children(parts);
    let code = div().w_full().overflow_hidden().px(px(10.)).py(px(7.)).rounded(sc(st.num("pre.radius"))).bg(st.color("pre.bg").unwrap_or_else(|| st.token("code_bg")))
        .border_1().border_color(st.color("pre.border").unwrap_or_else(|| st.token("code_border")))
        .font_family(mono.clone()).text_size(sc(st.num("pre.size"))).line_height(sc(st.num("pre.line"))).text_color(st.color("pre.color").unwrap_or_else(|| st.token("code_text")))
        .child(code_line("1", vec![tok("# ", syn("comment")), tok(tr("备份文档库", "back up the library"), syn("comment"))]))
        .child(code_line("2", vec![tok("cd ", syn("function")), tok("~/Documents ", text), tok("&& ", syn("keyword")), tok("tar ", syn("function")), tok("-czf ", syn("attr")), tok("\"Quiet.tar.gz\" ", syn("string")), tok("Quiet.md", text)]))
        .child(code_line("3", vec![tok("echo ", syn("function")), tok("\"done\"", syn("string"))]));
    // One column, like a page: a centred text column (the theme's width, scaled,
    // up to 760 so a wide frame isn't mostly margin) whose bottom fades into
    // the paper. Blocks come in the order the 开始指南 itself uses.
    let paper = st.color("page.bg").unwrap_or_else(|| st.token("bg_body"));
    // Its own bottom corners too: nothing clips it to the page's radius.
    let fade = div().absolute().left_0().right_0().bottom_0().h(px(56.)).rounded_b(px(7.)).bg(linear_gradient(180., linear_color_stop(paper.opacity(0.), 0.), linear_color_stop(paper, 1.)));
    // items_start: a column stretched to the frame would shrink its blocks to fit.
    div().relative().size_full().flex().justify_center().items_start().bg(paper).font_family(font).text_color(text).overflow_hidden()
        // min_w_0: the column follows the frame, not its widest unbreakable line.
        .child(div().w_full().min_w_0().overflow_hidden().max_w((sc(st.num("page.width")) * 1.35).min(px(760.))).px(px(32.)).pt(px(22.)).flex().flex_col().gap(gap)
            .child(div().text_size(sc(st.num("title.size"))).line_height(sc(st.num("title.size") * 1.25)).font_weight(FontWeight(st.num("title.weight")))
                .font_family(SharedString::from(st.font("title.font"))).text_color(st.color("title.color").unwrap_or(text)).child(tr("开始指南", "Getting started")))
            .child(para)
            .child(callout)
            .child(heading(1, tr("五分钟上手", "Five minutes")))
            .child(div().flex().flex_col().child(task(true, tr("打开这份指南", "Open this guide"))).child(task(false, tr("按 ⌘ N 新建一篇文档", "Press ⌘ N for a new document"))))
            .child(heading(2, tr("三种输入方式", "Three ways to write")))
            .child(div().flex().flex_col().child(bullet(tr("斜杠菜单：输入 / 插入任意内容块", "Slash menu: type / to insert any block"))).child(bullet(tr("Markdown：# 加空格变标题", "Markdown: # and a space make a heading"))))
            .child(div().flex().flex_col().child(number("1.", tr("先读「写作基础」", "Read Writing basics first"))).child(number("2.", tr("再看「内容块」", "Then Content blocks"))))
            .child(quote)
            .child(heading(3, tr("最常用的快捷键", "Shortcuts to know")))
            .child(table)
            .child(div().w_full().py(px(2.)).child(hr))
            .child(code))
        .child(fade)
}

fn badge(t: &themes::Theme) -> Option<&'static str> {
    if t.customized() { Some(tr("已修改", "Edited")) } else if t.bundled.is_none() { Some(tr("自定义", "Custom")) } else { None }
}

/// A pop-up button (macOS): the current choice and a chevron on `button`'s white pill.
fn popup_button(id: impl Into<ElementId>, label: impl Into<SharedString>, open: bool) -> Stateful<Div> {
    div().id(id).flex_none().max_w(px(220.)).h(px(28.)).pl(px(12.)).pr(px(8.)).flex().items_center().gap(px(6.)).rounded(px(8.)).cursor_pointer()
        .bg(if open { theme::c(craft::N100) } else { Theme::bg_body() }).border_1().border_color(Theme::button_border()).shadow(theme::shadow_button())
        .hover(|s| s.bg(theme::c(craft::N100))).active(|s| s.bg(theme::c(craft::N200)))
        .text_size(px(13.)).line_height(px(18.)).text_color(Theme::text())
        .child(div().min_w_0().truncate().child(label.into()))
        .child(ui::icon("chevron-down", 12., Theme::text_caption()))
}

fn apply_theme(w: &mut Workspace, id: String, cx: &mut Context<Workspace>) {
    let feishu = themes::themes().iter().find(|t| t.id == id).is_some_and(|t| t.style.feishu);
    w.theme_menu = None;
    w.set_pref(move |p| { p.theme = id; p.feishu = feishu; }, cx)
}

impl Workspace {
    /// 外观's theme dropdown: the themes in a list under the button, the one
    /// being looked at drawn beside it. Hovering a theme previews it (it stays
    /// previewed until another is hovered); clicking applies it.
    pub(crate) fn render_theme_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let previewed = self.theme_menu.clone()?;
        if self.view != View::Settings || self.settings_page != Page::Appearance { return None }
        let b = self.theme_anchor.get()?;
        let current = prefs().theme_id();
        let list = themes::themes();
        let shown = list.iter().find(|t| t.id == previewed).or_else(|| list.first())?;
        let about = match (shown.description.clone().filter(|d| !d.is_empty()), shown.author.clone().filter(|a| !a.is_empty() && shown.bundled.is_none())) {
            (Some(d), Some(a)) => Some(format!("{d} — {a}")),
            (d, a) => d.or(a),
        };
        let pane = div().flex_none().p(px(12.)).flex().flex_col().gap(px(10.))
            .child(div().w(px(CARD_W)).h(px(CARD_H)).rounded(px(8.)).overflow_hidden().border_1().border_color(Theme::line_border()).child(preview(shown, S).rounded(px(7.))))
            .child(div().w(px(CARD_W)).px(px(2.)).flex().flex_col().gap(px(2.))
                .child(div().text_size(px(13.)).line_height(px(18.)).font_weight(FontWeight::SEMIBOLD).child(shown.name().to_string()))
                .children(about.map(|a| div().text_size(px(12.)).line_height(px(17.)).text_color(Theme::text_caption()).child(a))));
        let rows: Vec<AnyElement> = list.iter().map(|t| {
            let g = SharedString::from(format!("theme-opt-{}", t.id));
            let (id, hover_id) = (t.id.clone(), t.id.clone());
            ui::menu_row(g.clone(), g.clone())
                // The one in the preview keeps a quiet fill once the pointer moves on.
                .when(t.id == shown.id, |d| d.bg(Theme::fill_hover()))
                .child(div().flex_1().min_w_0().overflow_hidden().child(ui::menu_label(t.name().to_string(), g.clone(), gpui::white())))
                .children(badge(t).map(|b| div().flex_none().text_size(px(11.)).text_color(Theme::text_caption()).child(ui::menu_label(b, g.clone(), gpui::white()))))
                .child(div().flex_none().w(px(14.)).when(t.id == current, |d| d.child(ui::menu_glyph("check", 14., g.clone()))))
                .on_hover(cx.listener(move |w, hovered: &bool, _, cx| if *hovered && w.theme_menu.as_ref() != Some(&hover_id) { w.theme_menu = Some(hover_id.clone()); cx.notify(); }))
                .on_click(cx.listener(move |w, _, _, cx| apply_theme(w, id.clone(), cx)))
                .into_any_element()
        }).collect();
        drop(list);
        let anchor = self.theme_anchor.clone();
        let menu = ui::popover().id("theme-menu").occlude().flex()
            .on_mouse_down_out(cx.listener(move |w, ev: &MouseDownEvent, _, cx| {
                // A press on the button itself: its click closes the menu.
                if anchor.get().is_some_and(|b| b.contains(&ev.position)) { return }
                w.theme_menu = None;
                cx.notify();
            }))
            .child(pane)
            .child(div().id("theme-menu-list").w(px(200.)).max_h(px(CARD_H + 96.)).overflow_y_scroll().py(px(ui::MENU_PAD_Y)).border_l_1().border_color(Theme::line_divider()).flex().flex_col().children(rows));
        Some(deferred(anchored().anchor(Corner::TopRight).position(point(b.right(), b.bottom() + px(4.))).snap_to_window_with_margin(px(8.)).child(menu)).with_priority(2).into_any_element())
    }

    /// Keys while the theme dropdown is open; true when used.
    pub(crate) fn theme_menu_key(&mut self, key: &str, cx: &mut Context<Self>) -> bool {
        let ids: Vec<String> = themes::themes().iter().map(|t| t.id.clone()).collect();
        let at = self.theme_menu.as_ref().and_then(|p| ids.iter().position(|id| id == p)).unwrap_or(0);
        match key {
            "escape" => self.theme_menu = None,
            "up" => self.theme_menu = Some(ids[at.saturating_sub(1)].clone()),
            "down" => self.theme_menu = Some(ids[(at + 1).min(ids.len().saturating_sub(1))].clone()),
            "enter" | "space" => { apply_theme(self, ids[at].clone(), cx); return true }
            _ => return false,
        }
        cx.notify();
        true
    }
}

impl Workspace {
    /// Change preferences and refresh what depends on them.
    pub(crate) fn set_pref(&mut self, f: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        let old = prefs();
        let mut p = old.clone();
        f(&mut p);
        if let Err(e) = save(&p) {
            self.show_toast_kind(ui::ToastKind::Error, &format!("{}: {e}", tr("设置保存失败", "Could not save settings")), cx);
            return;
        }
        if p.quick_hotkey != old.quick_hotkey {
            if p.quick_hotkey { crate::quick_note::register(cx); } else { crate::quick_note::suspend_hotkey(); }
        }
        if p.theme_id() != old.theme_id() || p.english != old.english {
            refresh_all(self, cx);
        }
        cx.notify();
    }

    pub(crate) fn reload_themes(&mut self, cx: &mut Context<Self>) {
        themes::load(&prefs().theme_id(), themes::all_loaded(), || cx.text_system().all_font_names());
        refresh_all(self, cx);
    }

    /// Open the theme's stylesheet in the user's editor; a built-in theme is
    /// first copied in full into the themes folder (the copy replaces it).
    fn edit_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        match themes::editable_file(id) {
            Ok(p) => {
                self.reload_themes(cx);
                open_in_editor(&p, cx);
            }
            Err(e) => self.show_toast_kind(ui::ToastKind::Error, &e.to_string(), cx),
        }
    }

    fn duplicate_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        match themes::duplicate(id) {
            Ok((new_id, p)) => {
                themes::load(&new_id, themes::all_loaded(), || cx.text_system().all_font_names());
                let feishu = themes::themes().iter().find(|t| t.id == new_id).is_some_and(|t| t.style.feishu);
                self.set_pref(move |pr| { pr.theme = new_id; pr.feishu = feishu; }, cx);
                open_in_editor(&p, cx);
            }
            Err(e) => self.show_toast_kind(ui::ToastKind::Error, &e.to_string(), cx),
        }
    }

    fn remove_theme(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Err(e) = themes::remove(id) { return self.show_toast_kind(ui::ToastKind::Error, &e.to_string(), cx) }
        self.reload_themes(cx);
        // A deleted theme of the user's own: back to Craft.
        if !themes::themes().iter().any(|t| t.id == id) { self.set_pref(|p| { p.theme = "craft".into(); p.feishu = false; }, cx); }
    }

    pub(crate) fn open_settings(&mut self, page: Page, cx: &mut Context<Self>) {
        self.cancel_shortcut_recording(cx);
        self.settings_page = page;
        self.theme_menu = None;
        self.settings_scroll.set_offset(point(px(0.), px(0.)));
        self.shortcut_error = None;
        if self.view != View::Settings { self.show_view(View::Settings, cx); }
        cx.notify();
    }

    pub(crate) fn render_settings_sidebar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let current = (self.view == View::Settings).then_some(self.settings_page);
        let in_trash = self.view == View::Trash;
        let nav_row = |page: Option<Page>, cx: &mut Context<Self>| {
            let on = if page.is_some() { page == current } else { in_trash };
            let (title, icon) = page.map_or((tr("回收站", "Trash"), "bin"), |p| (p.title(), p.icon()));
            div().id(SharedString::from(format!("settings-nav-{page:?}"))).h(px(sb::ROW_H)).flex().items_center().pl(px(6.)).pr(px(4.)).rounded(px(sb::ROW_R))
                .text_size(px(13.)).line_height(px(20.)).cursor_pointer()
                .when(on, |d| d.bg(Theme::sidebar_selected()).text_color(Theme::text()))
                .when(!on, |d| d.text_color(Theme::sidebar_text()).hover(|s| s.bg(Theme::sidebar_hover())))
                .child(div().size(px(20.)).flex_none().flex().items_center().justify_center().mr(px(9.)).child(ui::icon(icon, sb::ROW_ICON, if on { Theme::text_code() } else { Theme::icon() })))
                .child(title)
                .on_click(cx.listener(move |w, _, _, cx| match page { Some(p) => w.open_settings(p, cx), None => w.show_view(View::Trash, cx) }))
        };
        div().w(px(self.sidebar_w(window))).h_full().flex_none().flex().flex_col().text_color(Theme::text()).font_family(theme::UI_FONT)
            .child(div().h(px(crate::app::HEADER_H)).flex_none().flex().items_center().pl(px(theme::TRAFFIC_LIGHTS_W)).title_bar().child(self.settings_navigation(cx)))
            // The document sidebar's rhythm: 主页 sits 8 under the bar, a section
            // header like 我的文档's, rows on a 31 pitch. 设置's line sits on
            // 主页's, not a section header's: the 34pt box keeps the rows below.
            .child(div().pt(px(8.)).px(px(sb::SB_X)).flex().flex_col()
                .child(div().h(px(sb::SECTION_H)).pl(px(4.)).pt(px((sb::SECTION_TOP + sb::SECTION_H) / 2. - sb::ROW_H / 2.))
                    .child(div().px(px(4.)).text_size(px(13.)).line_height(px(20.)).font_weight(FontWeight::SEMIBOLD).text_color(Theme::sidebar_section()).child(tr("设置", "Settings"))))
                .child(div().flex().flex_col().gap(px(sb::ROW_GAP)).children(NAV.iter().map(|p| nav_row(*p, cx)).collect::<Vec<_>>())))
            // Where 设置 was: 返回, back to the page 设置 was opened from.
            .child(div().flex_1())
            .child(sb::footer_line())
            .child(sb::footer_slot(sb::footer_row("ft-back", "arrow-left", tr("返回", "Back")).tip(tr("返回", "Back"), Some("Esc"))
                .on_click(cx.listener(|w, _, _, cx| w.close_settings(cx)))))
            .into_any_element()
    }

    /// 收起 / 展开侧栏 and 主页, in the very box the document sidebar's
    /// 收起侧栏 uses (28pt, 4pt past the traffic lights, glyph 17), so the
    /// buttons don't move when switching between the document list and 设置.
    fn settings_navigation(&self, cx: &mut Context<Self>) -> AnyElement {
        let open = self.sidebar_open;
        let bar_button = |id: &'static str, glyph: &'static str| {
            div().id(id).no_drag().flex_none().size(px(28.)).flex().items_center().justify_center().rounded(px(8.)).cursor_pointer()
                .hover(move |s| s.bg(if open { Theme::sidebar_hover() } else { Theme::fill_hover_strong() }))
                .child(ui::icon(glyph, 17., Theme::icon()))
        };
        div().flex().items_center().ml(px(4.)).gap(px(4.))
            .child(bar_button("settings-collapse", if open { "panel-left-close" } else { "panel-left-open" })
                .tip(if open { tr("收起侧栏", "Hide sidebar") } else { tr("展开侧栏", "Show sidebar") }, Some(crate::shortcuts::hint("workspace::ToggleSidebar", "⌘ + ]")))
                .on_click(cx.listener(|w, _, _, cx| { w.sidebar_open = !w.sidebar_open; cx.notify(); })))
            .child(bar_button("settings-home", "home").tip(tr("返回主页", "Home"), Some(crate::shortcuts::hint("workspace::GoHome", "⌘ + ⌥ + Home")))
                .on_click(cx.listener(|w, _, _, cx| w.show_view(View::Home, cx))))
            .into_any_element()
    }

    /// Back to where 设置 was opened from.
    pub(crate) fn close_settings(&mut self, cx: &mut Context<Self>) {
        let back = if self.editor.is_some() && self.current.is_some() { View::Doc } else { View::Home };
        self.show_view(back, cx);
    }

    pub(crate) fn render_settings(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = self.settings_page;
        let body = match page {
            Page::General => self.general_page(cx),
            Page::Appearance => self.appearance_page(cx),
            Page::Shortcuts => self.shortcut_settings(cx),
            Page::Library => self.library_page(cx),
            Page::About => self.about_page(),
        };
        div().flex_1().min_w_0().h_full().flex().flex_col().bg(Theme::bg_body()).text_color(Theme::text()).font_family(theme::UI_FONT)
            // The bar only drags the window; the page title heads the centred column.
            .child(div().h(px(crate::app::HEADER_H)).flex_none().flex().items_center().title_bar()
                .when(!self.sidebar_open, |d| d.pl(px(theme::TRAFFIC_LIGHTS_W)).child(self.settings_navigation(cx))))
            .child(div().id("settings-content").flex_1().min_h_0().overflow_y_scroll().track_scroll(&self.settings_scroll)
                .child(div().w_full().flex().flex_col().items_center().px(px(32.)).pb(px(64.))
                    .child(div().w_full().max_w(px(if page == Page::Shortcuts { 760. } else { COLUMN_W })).flex().flex_col().gap(px(28.))
                        .child(div().flex().items_center().gap(px(16.))
                            .child(div().flex_1().min_w_0().px(px(4.)).text_size(px(22.)).line_height(px(28.)).font_weight(FontWeight::SEMIBOLD).child(page.title()))
                            .when(page == Page::Shortcuts, |d| d.child(self.shortcut_actions(cx))))
                        .child(body))))
            .into_any_element()
    }

    fn general_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let p = prefs();
        div().w_full().flex().flex_col().gap(px(28.))
            .child(group(None, vec![
                row(tr("界面语言", "Interface language"), None,
                    segmented("lang", &["简体中文", "English"], p.english as usize, |w, i, cx| w.set_pref(|p| p.english = i == 1, cx), cx)).into_any_element(),
                row(tr("自动下载网络图片", "Download web images"), desc(tr("关闭后不联网，网络图片显示为占位，点击单独加载。", "When off the app stays offline; web images show a placeholder you can click to load.")),
                    switch("pref-remote", p.remote_images).on_click(cx.listener(|w, _, _, cx| w.set_pref(|p| p.remote_images = !p.remote_images, cx)))).into_any_element(),
            ]))
            .into_any_element()
    }

    fn appearance_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let current = prefs().theme_id();
        let list = themes::themes();
        let sel = list.iter().find(|t| t.id == current).or_else(|| list.first());
        let (sel_id, sel_name, sel_path, customized, own, warnings) = match sel {
            Some(t) => (t.id.clone(), t.name().to_string(), t.path.clone(), t.customized(), t.bundled.is_none(), t.warnings.clone()),
            None => ("craft".into(), "Craft".into(), None, false, false, vec![]),
        };
        drop(list);
        let errors = themes::errors();
        let where_: SharedString = match &sel_path {
            // Inside the library: the short path (the full one wraps on narrow windows).
            Some(p) => p.strip_prefix(crate::store::Store::root_dir()).unwrap_or(p).display().to_string().into(),
            None => tr("内置主题", "Built in").into(),
        };
        let (edit_id, dup_id, rm_id, open_id) = (sel_id.clone(), sel_id.clone(), sel_id.clone(), sel_id);
        let anchor = self.theme_anchor.clone();
        let picker = popup_button("theme-picker", sel_name, self.theme_menu.is_some())
            .child(canvas(move |b, _, _| anchor.set(Some(b)), |_, _, _, _| {}).absolute().inset_0())
            .relative()
            .on_click(cx.listener(move |w, _, _, cx| {
                if w.theme_menu.is_none() && !themes::all_loaded() {
                    themes::load(&prefs().theme_id(), true, || cx.text_system().all_font_names());
                }
                w.theme_menu = if w.theme_menu.is_some() { None } else { Some(open_id.clone()) };
                cx.notify();
            }));
        // One group, top to bottom: the choice, how it looks, its stylesheet.
        // The preview is a page of the theme on a quiet band, like a sheet on a desk.
        let shown = { let list = themes::themes(); list.iter().find(|t| t.id == current).or_else(|| list.first()).map(|t| sample_page(t, PAGE_S).rounded(px(7.))) };
        let sample = div().w_full().px(px(16.)).py(px(16.)).flex().justify_center().bg(Theme::fill_hover())
            .child(div().w_full().h(px(360.)).rounded(px(8.)).overflow_hidden().border_1().border_color(Theme::line_border()).shadow(theme::shadow_button()).children(shown));
        div().w_full().flex().flex_col().gap(px(28.))
            .child(div().w_full().flex().flex_col().gap(px(8.))
                .child(group(None, vec![
                    row(tr("文档主题", "Document theme"), None, picker).into_any_element(),
                    sample.into_any_element(),
                    row(tr("主题 CSS", "Theme CSS"), Some(where_),
                        div().flex().items_center().gap(px(8.))
                            .when(customized || own, |d| d.child(link_button("theme-remove", if customized { tr("恢复默认", "Restore") } else { tr("删除", "Delete") })
                                .on_click(cx.listener(move |w, _, _, cx| w.remove_theme(&rm_id, cx)))))
                            .child(button("theme-duplicate", tr("复制为新主题", "Duplicate")).on_click(cx.listener(move |w, _, _, cx| w.duplicate_theme(&dup_id, cx))))
                            .child(button("theme-edit", tr("编辑 CSS", "Edit CSS")).on_click(cx.listener(move |w, _, _, cx| w.edit_theme(&edit_id, cx))))).into_any_element(),
                ]))
                .children(errors.into_iter().chain(warnings.into_iter().take(8).map(|w| format!("{}：{w}", tr("未生效", "Ignored")))).map(|e| div().px(px(4.)).text_size(px(12.)).line_height(px(18.)).text_color(Theme::danger()).child(e)))
                )
            .into_any_element()
    }

    fn library_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let root = crate::store::Store::root_dir();
        div().w_full().flex().flex_col().gap(px(8.))
            .child(group(None, vec![
                // Under the home folder as ~/…, the way Finder and other apps write it.
                row(tr("文档库文件夹", "Library folder"), Some(match dirs::home_dir().and_then(|h| root.strip_prefix(h).ok().map(|r| format!("~/{}", r.display()))) { Some(p) => p, None => root.display().to_string() }.into()),
                    button("reveal-library", tr("在访达中显示", "Show in Finder")).on_click(|_, _, cx| cx.reveal_path(&crate::store::Store::root_dir()))).into_any_element(),
                row(tr("回收站", "Trash"), Some(if english() { format!("Deleted documents are kept for {} days.", crate::store::TRASH_DAYS) } else { format!("删除的文档保留 {} 天后自动清除。", crate::store::TRASH_DAYS) }.into()),
                    button("open-trash", tr("打开", "Open")).on_click(cx.listener(|w, _, _, cx| w.show_view(View::Trash, cx)))).into_any_element(),
            ]))
            .into_any_element()
    }

    fn about_page(&self) -> AnyElement {
        div().w_full().pt(px(24.)).flex().flex_col().items_center().gap(px(10.))
            .child(img(about_icon()).size(px(88.)))
            .child(div().text_size(px(20.)).line_height(px(26.)).font_weight(FontWeight::SEMIBOLD).child("quiet.md"))
            .child(div().text_size(px(13.)).line_height(px(18.)).text_color(Theme::text_caption()).child(format!("{} {}", tr("版本", "Version"), env!("CARGO_PKG_VERSION"))))
            .child(div().text_size(px(13.)).line_height(px(20.)).text_color(Theme::text_caption()).child(tr("本地优先的笔记应用：无需账号，文档就是你电脑上的文件。", "Local-first notes: no account, your documents are files on your Mac.")))
            .into_any_element()
    }
}

/// The app icon resampled once to 176px, its size at 88pt on a 2x screen.
/// GPUI samples images bilinearly without mipmaps, so the 1024px file shrunk
/// 5.8× on the GPU skips most of its pixels and the edges come out jagged.
/// On a 1x screen the 2:1 step is still an exact box filter.
fn about_icon() -> ImageSource {
    const PATH: &str = "branding/app-icon.png";
    static ICON: LazyLock<Option<Arc<RenderImage>>> = LazyLock::new(|| {
        let bytes = crate::assets::Assets.load(PATH).ok()??;
        let mut buf = image::imageops::resize(&image::load_from_memory(&bytes).ok()?.into_rgba8(), 176, 176, image::imageops::FilterType::Lanczos3);
        for px in buf.pixels_mut() {
            px.0.swap(0, 2);
        }
        Some(Arc::new(RenderImage::new(vec![image::Frame::new(buf)])))
    });
    match &*ICON { Some(icon) => icon.clone().into(), None => PATH.into() }
}

pub fn label(text: impl AsRef<str>) -> String {
    let text = text.as_ref();
    if !english() { return text.to_string(); }
    match text {
        "主页" => "Home".to_string(),
        "我的文档" => "My documents".to_string(),
        "置顶" => "Pinned".to_string(),
        "收藏" => "Favorites".to_string(),
        "设置" => "Settings".to_string(),
        "新建" => "New".to_string(),
        "新建文档" => "New document".to_string(),
        "搜索" => "Search".to_string(),
        "搜索文档…" => "Search documents…".to_string(),
        "搜索文档..." => "Search documents...".to_string(),
        "最近访问" => "Recently visited".to_string(),
        "最近访问时间" => "Last visited".to_string(),
        "最近修改时间" => "Last modified".to_string(),
        "创建时间" => "Date created".to_string(),
        "修改时间" => "Date modified".to_string(),
        "文档名称" => "Document name".to_string(),
        "名称" => "Name".to_string(),
        "回收站" => "Trash".to_string(),
        "存储管理" => "Storage".to_string(),
        "更多" => "More".to_string(),
        "返回" => "Back".to_string(),
        "取消" => "Cancel".to_string(),
        "确定" => "OK".to_string(),
        "删除" => "Delete".to_string(),
        "移到回收站" => "Move to trash".to_string(),
        "恢复" => "Restore".to_string(),
        "清空回收站" => "Empty trash".to_string(),
        "永久删除" => "Delete permanently".to_string(),
        "彻底删除" => "Delete permanently".to_string(),
        "重命名" => "Rename".to_string(),
        "创建副本" => "Duplicate".to_string(),
        "复制" => "Copy".to_string(),
        "粘贴" => "Paste".to_string(),
        "剪切" => "Cut".to_string(),
        "全选" => "Select all".to_string(),
        "导入" => "Import".to_string(),
        "导出" => "Export".to_string(),
        "下载" => "Download".to_string(),
        "移动到" => "Move to".to_string(),
        "添加到收藏" => "Add to favorites".to_string(),
        "取消收藏" => "Remove from favorites".to_string(),
        "取消置顶" => "Unpin".to_string(),
        "文档信息" => "Document information".to_string(),
        "字数统计" => "Word count".to_string(),
        "快捷键" => "Keyboard shortcuts".to_string(),
        "最近浏览" => "Recently viewed".to_string(),
        "所有文档" => "All documents".to_string(),
        "按名称" => "By name".to_string(),
        "上次浏览" => "Last viewed".to_string(),
        "创建" => "Created".to_string(),
        "修改" => "Modified".to_string(),
        "访问" => "Visited".to_string(),
        "收藏时间" => "Date favorited".to_string(),
        "文档标题" => "Document title".to_string(),
        "暂无文档" => "No documents yet".to_string(),
        "没有找到相关文档" => "No documents found".to_string(),
        "暂无收藏文档" => "No favorites yet".to_string(),
        "收藏的文档会显示在这里" => "Your favorites will appear here".to_string(),
        "左对齐" => "Align left".to_string(),
        "居中对齐" => "Align center".to_string(),
        "右对齐" => "Align right".to_string(),
        "正文" => "Text".to_string(),
        "一级标题" => "Heading 1".to_string(),
        "二级标题" => "Heading 2".to_string(),
        "三级标题" => "Heading 3".to_string(),
        "四级标题" => "Heading 4".to_string(),
        "五级标题" => "Heading 5".to_string(),
        "六级标题" => "Heading 6".to_string(),
        "加粗" => "Bold".to_string(),
        "斜体" => "Italic".to_string(),
        "下划线" => "Underline".to_string(),
        "删除线" => "Strikethrough".to_string(),
        "行内代码" => "Inline code".to_string(),
        "链接" => "Link".to_string(),
        "代码块" => "Code block".to_string(),
        "无序列表" => "Bulleted list".to_string(),
        "有序列表" => "Numbered list".to_string(),
        "任务" => "Task".to_string(),
        "待办事项" => "To-do list".to_string(),
        "引用" => "Quote".to_string(),
        "分割线" => "Divider".to_string(),
        "高亮块" => "Callout".to_string(),
        "图片" => "Image".to_string(),
        "表格" => "Table".to_string(),
        "分栏" => "Columns".to_string(),
        "基础" => "Basic".to_string(),
        "常用" => "Common".to_string(),
        "更多类型" => "More blocks".to_string(),
        "文字颜色" => "Text color".to_string(),
        "背景颜色" => "Background color".to_string(),
        "边框颜色" => "Border color".to_string(),
        "添加图标" => "Add icon".to_string(),
        "添加封面" => "Add cover".to_string(),
        "编辑封面" => "Edit cover".to_string(),
        "移除封面" => "Remove cover".to_string(),
        "自动换行" => "Wrap lines".to_string(),
        "取消换行" => "Unwrap lines".to_string(),
        "文档库内搜索" => "Search documents".to_string(),
        "网页搜索" => "Search the web".to_string(),
        "展开" => "Expand".to_string(),
        "收起" => "Collapse".to_string(),
        "选择语言" => "Choose language".to_string(),
        "视频或文件" => "Video or file".to_string(),
        "清除格式" => "Clear formatting".to_string(),
        "转换为" => "Turn into".to_string(),
        "向上插入" => "Insert above".to_string(),
        "向下插入" => "Insert below".to_string(),
        "保存" => "Save".to_string(),
        "关闭" => "Close".to_string(),
        _ => text.to_string(),
    }
}
