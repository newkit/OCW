#!/usr/bin/env python3
"""Writes the face-* scenes of scripts/scenes.toml to stdout (spec 14
Phase 4, 4N Faces): nine made-up riders - skin tones, ages, base bodies,
every beard and moustache, no glasses / sport sunglasses / round glasses -
each from the front, three-quarter and from the side, six of them with
the helmet off in the character editor, and a group at riding distance.
The same looks render before and after a change of the faces."""

R = [
    # key, name, kg, cm, bike, power, character, kit, text
    ("a", "Ada Rider", 58, 168, "road", "150",
     "--body b --shape skinny --age 20s --hair long --hair-colour auburn --skin 1 --glasses 1",
     "--pattern plain --jersey #1E88E5 --second #F2F2EE --helmet #E53935",
     "base body B, 20s, fair skin, long auburn hair, clean-shaven, no glasses"),
    ("b", "Cyd Sprinter", 80, 182, "road-aero", "150",
     "--body c --shape normal --age 40s --hair crew-cut --hair-colour #3B2A20 --skin 11 --beard short-boxed --glasses 2",
     "--pattern stripes --jersey #43A047 --second #F2F2EE --helmet #F2F2EE",
     "base body C, 40s, brown skin, crew cut, short boxed beard, sport sunglasses"),
    ("c", "Dag Tourer", 96, 176, "gravel", "140",
     "--body a --shape ample --age 70+ --hair bald --skin 5 --beard full --moustache classic --glasses 3",
     "--pattern sash --jersey #FB8C00 --second #3949AB --helmet #3949AB",
     "base body A, 70+, light skin, bald, grey full beard and classic moustache, round glasses"),
    ("d", "Eve Tourer", 70, 170, "road", "150",
     "--body b --shape normal --age 60s --hair low-bun --hair-colour black --skin 14 --glasses 3",
     "--pattern chevrons --jersey #7E57C2 --second #FDD835 --helmet #F2F2EE",
     "base body B, 60s, dark skin, greying low bun, clean-shaven, round glasses"),
    ("e", "Finn Gravel", 72, 180, "gravel", "150",
     "--body a --shape normal --age 30s --hair short-textured --hair-colour brown --skin 8 --beard stubble --glasses 1",
     "--pattern plain --jersey #00897B --second #F2F2EE --helmet #212121",
     "base body A, 30s, tan skin, short textured hair, stubble, no glasses"),
    ("f", "Gus Climber", 64, 174, "road", "150",
     "--body c --shape skinny --age 50s --hair side-part --hair-colour light-brown --skin 3 --beard chin-strap --moustache pencil --glasses 2",
     "--pattern stripes --jersey #D81B60 --second #212121 --helmet #D81B60",
     "base body C, 50s, light skin, side part, chin-strap beard and pencil moustache, sport sunglasses"),
    ("g", "Hal Rouleur", 77, 184, "road", "150",
     "--body a --shape normal --age 30s --hair buzz-cut --hair-colour black --skin 9 --beard goatee --moustache handlebar --glasses 1",
     "--pattern sash --jersey #FDD835 --second #212121 --helmet #212121",
     "base body A, 30s, olive-brown skin, buzz cut, goatee and handlebar moustache, no glasses"),
    ("h", "Ike Puncheur", 84, 178, "road", "150",
     "--body c --shape ample --age 40s --hair shaved --hair-colour red --skin 6 --beard long --moustache chevron --glasses 1",
     "--pattern plain --jersey #5D4037 --second #F2F2EE --helmet #F2F2EE",
     "base body C, 40s, light-tan skin, shaved head, long red beard and chevron moustache, no glasses"),
    ("i", "Jo Baroudeur", 62, 172, "road", "150",
     "--body b --shape skinny --age 20s --hair afro --hair-colour black --skin 15 --beard short-stubble-beard --moustache horseshoe --glasses 1",
     "--pattern chevrons --jersey #F4511E --second #F2F2EE --helmet #F2F2EE",
     "base body B, 20s, very dark skin, afro, short stubble beard and horseshoe moustache, no glasses"),
]
V = [
    ("front", 180, 0, "from the front"),
    ("threeq", 140, -4, "three-quarter"),
    ("side", 90, 0, "from the side"),
]


def riders(n, kg, cm, bike):
    return (f'active = \\"{n}\\"\\n[[rider]]\\nname = \\"{n}\\"\\n'
            f'weight_kg = {kg}.0\\nheight_cm = {cm}.0\\nbike = \\"{bike}\\"\\n')


print("""
# Faces (4N Faces, 2026-11-28: eyes, lips and beards): the same looks before
# and after, front / three-quarter / side, the editor's bare-head close-up
# and a group at riding distance. Made-up rider names only.
# Written by scripts/face-scenes.py.""")
for k, n, kg, cm, bike, pw, ch, kit, text in R:
    for v, yaw, pitch, vt in V:
        print(f'''
[[scene]]
name = "face-{k}-{v}"
description = "Faces (4N): {n}, {vt} - {text}"
args = ["--route", "hills", "--bots", "0", "--power", "{pw}"]
riders = "{riders(n, kg, cm, bike)}"
at = ["0.5=time 2026-06-21 15:00", "0.5=hud off", "0.5=hud.profile off", "1.5=character {ch}", "1.6=kit.design {kit}", "4=camera.closeup face {yaw} {pitch} 0.38"]
shot = 9
pick = false''')
for k, n, kg, cm, bike, pw, ch, kit, text in R[:6]:
    print(f'''
[[scene]]
name = "face-{k}-bare"
description = "Faces (4N): {n}, the helmet off in the character editor's face close-up - {text}"
args = ["--route", "hills", "--bots", "0"]
riders = "{riders(n, kg, cm, bike)}"
at = ["0.5=time pause", "0.5=time 2026-06-20 13:00", "2=character {ch}", "2.4=menu workshop-page", "2.8=menu down", "3.2=menu down", "3.6=menu select", "4.0=menu down", "4.4=menu select"]
shot = 9
pick = false''')
k, n, kg, cm, bike, pw, ch, kit, text = R[1]
print(f'''
[[scene]]
name = "face-ride"
description = "Faces (4N): riding distance - {n} on the Hills route with six bots in their own looks (orbit camera 4 m in front): the faces at the distance of a ride, the frame-rate overlay on"
args = ["--route", "hills", "--bots", "6", "--power", "220"]
riders = "{riders(n, kg, cm, bike)}"
at = ["0.5=time 2026-06-21 15:00", "0.5=hud off", "0.5=hud.profile off", "1.5=character {ch}", "1.6=kit.design {kit}", "4=camera.orbit 160 8 4"]
shot = 12
pick = false''')
