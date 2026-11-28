# Reference: the folding bike (spec 2.5, 14 Phase 15; 2026-11-28)

The model `artwork/3dmodels/bikes/bike-folding.glb` (profile `folding`) was
made after **a photo the user took and shared on 2026-11-28** ("here is a
foto of a foldable bike, use that as a new model. use the color schema").
The photo is the user's, for reference only: it is **not** in the
repository (it shows a real place and a tyre brand). Nothing of it is
copied – the model is generated from geometry numbers
(`ocw_assets::bikes::design(ProfileId::Folding)`).

What the photo shows and what the model takes from it:

| Feature in the photo | In the model |
|---|---|
| A classic 1970s–80s 20-inch folding bike | 20 × 1.75 wheels (radius 0.247 m), 28 spokes, bottom bracket 3.5 cm above the axles |
| One main tube, step-through, a fold hinge with a quick-release lever in its middle | `folding_main_tube` from low on the head tube to low on the seat tube; the hinge block in the frame's paint, a chrome lever; node `fold_hinge` |
| Tall chrome handlepost leaning back, its own fold clamp, swept-back city bar with a bell | `spacers` 0.22 m of chrome post, a collar, `Bars::Swept` (580 mm, 80 mm rise, 50 mm sweep back), a bell on the left |
| Sky-blue frame and fork | the default paint `#6FB3E0` (`ProfileId::default_paint`); any palette colour can be chosen |
| Hot pink tyres, grips and pedals | slots `tyre_colour` and `grip`, `#FF3FAE` in the file; **plain tyres, no lettering** (the photo's tyres carry a brand name: not copied) |
| Silver rims with spokes | rims in `metal` |
| Chrome fenders front and rear with a red stripe | `chrome` fenders with a `paint_red` stripe, one stay per side |
| Rear rack with a red reflector | the rack painted like the frame (it is light blue in the photo), the rear lamp at its end |
| Black sprung saddle on a long seat post | a broader saddle with two coil springs, a long chrome post |
| One chainring with a chainguard | one 85 mm ring, one sprocket, no derailleur, a chrome guard |
| Bottle dynamo and a small front lamp | a chrome dynamo on the right fork blade, a box lamp on the head tube |
