use super::*;
use fuze_capture::pick_pixels::PixelStrip;
use fuze_contract::{titles::Region, Layout};

const SCREEN: (u32, u32) = (852, 480);

struct Scene(Vec<u8>);

impl Scene {
    fn new(layout: Layout, held: &[char], waiting: bool) -> Self {
        let (w, h) = SCREEN;
        let mut scene = Self(vec![10; (w * h * 4) as usize]);
        for (i, letter) in ['O', 'T', 'L', 'X'][..layout.count()].iter().enumerate() {
            let regions = fuze_contract::titles::regions_for(w, h, layout).unwrap();
            let center = match layout {
                Layout::Three => w as f32 * (0.309 + 0.191 * i as f32),
                Layout::Four => regions[i].x as f32 + regions[i].width as f32 / 2.,
            };
            scene.draw(center, 137., 40., *letter);
        }
        for (i, letter) in held
            .iter()
            .copied()
            .chain(waiting.then_some('?'))
            .enumerate()
        {
            scene.draw(
                216.7 + 23. * (i % 3) as f32,
                441.82 + 23. * (i / 3) as f32,
                16.1,
                letter,
            );
        }
        scene
    }

    fn draw(&mut self, cx: f32, cy: f32, side: f32, letter: char) {
        for y in (cy - side / 2.).ceil() as u32..(cy + side / 2.).ceil() as u32 {
            for x in (cx - side / 2.).ceil() as u32..(cx + side / 2.).ceil() as u32 {
                let col = (((x as f32 - cx) / side + 0.5) * 12.) as i32;
                let row = (((y as f32 - cy) / side + 0.5) * 12.) as i32;
                let run = (2..=9).contains(&col) && (2..=9).contains(&row);
                let lit = run
                    && match letter {
                        'O' => (col - 5).pow(2) + (row - 5).pow(2) >= 8,
                        'T' => row < 4 || (5..=6).contains(&col),
                        'L' => col < 4 || row > 7,
                        'X' => (col - row).abs() <= 1 || (col + row - 11).abs() <= 1,
                        _ => false,
                    };
                let value = if letter == '?' {
                    (15 + hud::MARKER[(row * 12 + col) as usize]).max(0) as u8
                } else if lit {
                    185
                } else {
                    10
                };
                let ix = (y * SCREEN.0 + x) as usize * 4;
                self.0[ix..ix + 3].fill(value);
            }
        }
    }

    fn pixels(&self) -> PickPixels<'_> {
        pixels(&self.0, SCREEN)
    }
}

fn pixels(bytes: &[u8], screen: (u32, u32)) -> PickPixels<'_> {
    let (width, height) = screen;
    let plane = PixelStrip::new(
        bytes,
        width as usize * 4,
        Region {
            x: 0,
            y: 0,
            width,
            height,
        },
    )
    .unwrap();
    PickPixels {
        screen,
        hud: plane,
        cards: plane,
    }
}

fn cards<const N: usize>(ids: [u32; N]) -> CardSlots<Slot>
where
    CardSlots<Slot>: From<[Slot; N]>,
{
    ids.map(|id| Slot::Known {
        id,
        name: format!("符文{id}"),
        confidence: 1.,
    })
    .into()
}

fn point(screen: (u32, u32), layout: Layout, slot: usize) -> (i32, i32) {
    let r = pick_regions(screen.0, screen.1, layout).unwrap()[slot];
    ((r.x + r.width / 2) as i32, (r.y + r.height / 2) as i32)
}

fn click(t: &mut Tracker, p: (i32, i32), at: u64) {
    t.pointer(true, Some(p), at);
    t.pointer(false, Some(p), at + 20);
}

fn show(t: &mut Tracker, slots: &CardSlots<Slot>, scene: PickPixels<'_>, at: u64) {
    t.show(
        slots,
        TitleKind::Candidate,
        scene.screen,
        at,
        hud::Offer::read(scene, slots.layout(), None),
    );
}

fn feed(t: &mut Tracker, evidence: Evidence, scene: PickPixels<'_>, at: u64) {
    if let Some(pending) = t.frame(Some(evidence), at) {
        if pending.matches(scene) {
            t.confirm(&pending);
        }
    }
}

#[test]
fn fourth_card_confirms_with_second_title_missing_and_next_offer_visible() {
    let mut t = Tracker::default();
    let mut slots = cards([1047, 3116, 3060, 1996]);
    slots[1] = Slot::Unreadable;
    let before = Scene::new(Layout::Four, &['L'], true);
    let after = Scene::new(Layout::Three, &['X', 'L'], true);
    show(&mut t, &slots, before.pixels(), 100);
    let fourth = point(SCREEN, Layout::Four, 3);
    t.pointer(true, Some(fourth), 200);
    // The game changes layout before the mouse goes up. Keep the press's old
    // geometry and baseline, even when the latest offer is a skill choice.
    t.show(
        &cards([1004, 2018, 3060]),
        TitleKind::SkillChoice,
        SCREEN,
        210,
        None,
    );
    t.pointer(false, Some(fourth), 220);
    feed(&mut t, Evidence::Visible, after.pixels(), 215);
    assert!(t.owned.is_empty());
    feed(&mut t, Evidence::Visible, after.pixels(), 400);
    assert_eq!(&*t.owned, &[(1996, "符文1996".into())]);

    let snapshot = crate::stage_reference::Snapshot {
        schema: 1,
        champion: 145,
        patch: "16.22".into(),
        date: "2026-11-06".into(),
        data_path: String::new(),
        fetched: 100,
        rows: Default::default(),
        combinations: Some(vec![crate::stage_reference::Combination {
            ids: vec![1996, 3086],
            win: 0.6495,
            samples: 6819,
        }]),
    };
    let combo =
        crate::augment_combos::reference(Some(&snapshot), 145, "16.22", &t.owned, 3086, 101);
    assert_eq!(combo.partners, "符文1996");
}

#[test]
fn hide_reopen_and_reroll_leave_the_inventory_unchanged_before_a_three_card_pick() {
    let mut t = Tracker::default();
    let slots = cards([1004, 2018, 3060]);
    let before = Scene::new(Layout::Three, &[], true);
    let after = Scene::new(Layout::Three, &['T'], false);
    show(&mut t, &slots, before.pixels(), 100);
    let hide = (426, 380);
    click(&mut t, hide, 200);
    show(&mut t, &slots, before.pixels(), 190); // in-flight OCR from before the hide
    assert!(t.offer.is_none());
    feed(&mut t, Evidence::Absent, before.pixels(), 400);
    click(&mut t, hide, 600);
    show(&mut t, &slots, before.pixels(), 700);
    click(&mut t, (426, 340), 800);
    feed(&mut t, Evidence::Partial, before.pixels(), 1000);
    assert_eq!(t.diagnostics.clicks, 0);
    assert!(t.owned.is_empty());
    show(&mut t, &slots, before.pixels(), 1200);
    click(&mut t, point(SCREEN, Layout::Three, 1), 1300);
    feed(&mut t, Evidence::Absent, after.pixels(), 1450);
    assert_eq!(&*t.owned, &[(2018, "符文2018".into())]);
}

#[test]
fn closing_or_showing_an_existing_or_unrelated_icon_cannot_confirm_a_click() {
    let slots = cards([1004, 2018, 3060]);
    let before = Scene::new(Layout::Three, &['O'], true);
    let wrong = Scene::new(Layout::Three, &['O', 'L'], false);
    let missing = Scene::new(Layout::Three, &[], false);
    let moved_only = Scene::new(Layout::Three, &['O'], false);
    let mut stats = Scene::new(Layout::Three, &['O', 'T'], false);
    stats.draw(228.2, 441.82, 16., 'O');
    let mut t = Tracker::default();
    show(&mut t, &slots, before.pixels(), 100);
    click(&mut t, point(SCREEN, Layout::Three, 1), 200);
    assert!(t.pending.is_some());
    for (i, frame) in [&before, &wrong, &missing, &moved_only, &stats]
        .into_iter()
        .enumerate()
    {
        feed(
            &mut t,
            Evidence::Absent,
            frame.pixels(),
            350 + i as u64 * 350,
        );
        assert!(t.owned.is_empty());
    }
    assert_eq!(t.capture_at(Some(9000), 2500, 2000), Some(2500));
    assert_eq!(t.capture_at(None, 2500, 2000), None);
    assert_eq!(t.capture_at(Some(9000), 2800, 2500), Some(9000));
    assert_eq!(t.diagnostics.reason, Reason::HudTimeout);
}

#[test]
fn unreadable_hud_drag_foreign_input_and_stale_offer_never_confirm() {
    let slots = cards([1004, 2018, 3060]);
    let before = Scene::new(Layout::Three, &[], true);
    let no_marker = Scene::new(Layout::Three, &[], false);
    let after = Scene::new(Layout::Three, &['O'], false);
    let target = point(SCREEN, Layout::Three, 0);
    for scenario in 0..5 {
        let mut t = Tracker::default();
        show(
            &mut t,
            &slots,
            if scenario == 0 {
                no_marker.pixels()
            } else {
                before.pixels()
            },
            100,
        );
        match scenario {
            1 => click(&mut t, target, 1100),
            2 => {
                t.pointer(true, Some(target), 200);
                t.pointer(false, Some(point(SCREEN, Layout::Three, 1)), 220);
            }
            3 => {
                click(&mut t, target, 200);
                t.pointer(true, None, 250);
            }
            4 => {
                click(&mut t, target, 200);
                t.lost_input(250);
            }
            _ => click(&mut t, target, 200),
        }
        feed(&mut t, Evidence::Absent, after.pixels(), 1300);
        assert!(t.owned.is_empty(), "scenario {scenario}");
    }
}

#[test]
fn pandoras_box_releases_stale_combination_inputs_after_the_icons_are_replaced() {
    let mut recorder = Recorder::default();
    recorder.restore(Arc::new(vec![(1004, "符文1004".into())]));
    let previous = recorder.owned();
    let slots = cards([PANDORAS_BOX, 2018, 3060]);
    let before = Scene::new(Layout::Three, &['T'], true);
    let after = Scene::new(Layout::Three, &['X'], false);
    let mut t = recorder.shared.lock().unwrap();
    show(&mut t, &slots, before.pixels(), 100);
    click(&mut t, point(SCREEN, Layout::Three, 0), 200);
    feed(&mut t, Evidence::Absent, before.pixels(), 300);
    assert!(Arc::ptr_eq(&previous, &t.owned));
    feed(&mut t, Evidence::Absent, after.pixels(), 800);
    assert!(t.owned.is_empty());
    drop(t);
    let mut recovery = Recorder::default();
    recovery.restore(recorder.owned());
    assert!(recovery.owned().is_empty());
}

#[test]
fn hit_regions_exclude_reroll_buttons_for_both_layouts() {
    for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440)] {
        for slots in [cards([1, 2, 3]), cards([1, 2, 3, 4])] {
            let mut t = Tracker::default();
            t.show(&slots, TitleKind::Candidate, (w, h), 100, None);
            let offer = t.offer.unwrap();
            for i in 0..slots.len() {
                let (x, y) = point((w, h), slots.layout(), i);
                assert_eq!(offer.slot(x, y), Some(i));
            }
            for b in fuze_contract::selection_pixels::buttons_for(w, h, slots.layout())
                .unwrap()
                .iter()
            {
                assert_eq!(
                    offer.slot((b.x + b.width / 2) as i32, (b.y + b.height / 2) as i32),
                    None
                );
            }
        }
    }
}
