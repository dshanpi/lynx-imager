"""Compare rendered workspace chrome in the same Ubuntu native window."""
import argparse
import json
from pathlib import Path

from native_driver import launch, wait


def measure(web, selectors):
    return web.js("""
        const box = e => {
            const r = e.getBoundingClientRect();
            return [r.x, r.y, r.width, r.height];
        };
        const [rail, heading, main, content, title, footer] =
            arguments[0].map(s => document.querySelector(s));
        return {
            window: [innerWidth, innerHeight],
            rail: box(rail), heading: box(heading),
            mainX: box(main)[0], contentOrigin: box(content).slice(0, 2),
            titleSize: getComputedStyle(title).fontSize,
            headingSize: getComputedStyle(heading).fontSize,
            bodySize: getComputedStyle(main).fontSize,
            family: getComputedStyle(main).fontFamily,
            footer: box(footer),
            header: [...document.querySelectorAll('.imager-brand, .imager-app-header button')].map(box),
            overflow: [document.documentElement, content].map(e => e.scrollWidth - e.clientWidth)
        };
    """, selectors)


def inspect(binary, out):
    report = []
    with launch(binary) as (web, _):
        for width, height in [(1280, 768), (1090, 720), (1440, 900)]:
            web.call("POST", web.session + "/window/rect", {"width": width, "height": height})
            wait(lambda: web.js("return innerWidth === arguments[0]", width), "window resized")
            results = {}
            for page, name, selectors in [
                ("高级功能", "advanced", [".board-list", ".device-board-rail-heading h3", ".board-main",
                 ".adv-no-target", ".adv-no-target h2", ".imager-app-status"]),
                ("烧录系统", "basic", [".v2-rail", ".v2-rail-title h2", ".v2-content",
                 ".target-heading", ".target-page-heading h2", ".v2-foot"]),
            ]:
                web.click('.imager-app-tabs button', page)
                wait(lambda: web.find(selectors[3]), page)
                results[name] = measure(web, selectors)
                if width == 1280:
                    web.shot(out / (name + ".png"))
            basic, advanced = results["basic"], results["advanced"]
            for key in ("rail", "mainX", "contentOrigin", "headingSize", "titleSize", "family",
                        "bodySize", "footer", "header"):
                assert basic[key] == advanced[key], (width, key, basic[key], advanced[key])
            assert basic["heading"][:2] == advanced["heading"][:2], results
            assert max(basic["overflow"]) <= 1, basic
            web.click('.imager-app-tabs button', '连接 AI')
            wait(lambda: web.find('.im-ai-rail'), 'AI workspace')
            assert web.js("return document.querySelector('.im-ai-rail').getBoundingClientRect().width") == basic["mainX"]
            report.append(results)
            print(width, height, 'aligned', basic["mainX"], basic["contentOrigin"], flush=True)
    (out / "layout.json").write_text(json.dumps(report, ensure_ascii=False, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('binary', type=Path)
    parser.add_argument('output', type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    inspect(args.binary, args.output)
