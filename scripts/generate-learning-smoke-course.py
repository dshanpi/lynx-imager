#!/usr/bin/env python3
"""Generate an explicitly synthetic, signed local course for native acceptance.

The ephemeral signing key never leaves memory. No official signing configuration,
website service, learner account, or hardware is used. Requires cryptography.
"""

import argparse
import base64
import copy
import hashlib
import io
import json
from pathlib import Path
import zipfile

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


def encoded(value):
    return json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode()


def digest(value):
    return hashlib.sha256(value).hexdigest()


def generate(destination, scenario="c"):
    if scenario not in ("c", "multimedia", "theory", "mixed", "bsp", "optional", "hil-trust", "environment", "environment-execution", "prerequisites"):
        raise ValueError("Unknown course scenario")
    destination.mkdir(parents=True, exist_ok=False)
    key = Ed25519PrivateKey.generate()
    sources = {
        "src/main.c": b'#include <stdio.h>\nint main(void) { puts("LYNX local course: 42"); return 0; }\n',
        "scripts/verify.sh": b'''#!/bin/sh
set -eu
case "$1" in
  source) test -f src/main.c; printf 'source-present\\n' ;;
  build) mkdir -p build; cc -Wall -Wextra -Werror src/main.c -o build/lesson ;;
  run) result=$(./build/lesson); printf '%s\\n' "$result"; test "$result" = 'LYNX local course: 42' ;;
  *) exit 64 ;;
esac
''',
    }
    if scenario == "multimedia":
        sources = {
            "scripts/verify.sh": b'#!/bin/sh\nset -eu\nexec python3 scripts/media_lesson.py "$@"\n',
            "scripts/media_lesson.py": Path(__file__).with_name("learning-media-fixture.py").read_bytes(),
            "settings.json": encoded({"codec": "flac", "sampleRate": 16000, "channels": 1}),
        }
    if scenario in ("bsp", "environment", "environment-execution"):
        sources = {
            "scripts/verify.sh": b'#!/bin/sh\nset -eu\nexec python3 scripts/bsp_lesson.py "$@"\n',
            "scripts/bsp_lesson.py": Path(__file__).with_name("learning-bsp-fixture.py").read_bytes(),
            "src/main.c": b'const struct { unsigned int signature; unsigned int memory_bytes; } lynx_bsp_config = {0x4c594e58U, 64U * 1024U * 1024U};\n',
            "src/board.dts": b'''/dts-v1/;
/ {
    model = "LYNX synthetic BSP fixture";
    compatible = "lynx,bsp-fixture";
    #address-cells = <1>;
    #size-cells = <1>;
    memory@40000000 {
        device_type = "memory";
        reg = <0x40000000 0x04000000>;
    };
};
''',
        }
    if scenario in ("environment", "environment-execution"):
        sources["sdk/VERSION"] = b"LYNX synthetic BSP SDK revision 1 - not a vendor SDK\n"
    if scenario == "environment-execution":
        sources["scripts/verify.sh"] = b'#!/bin/sh\nset -eu\nif [ "$1" = source ] && [ -f ../shared-sdk/change-during-run ]; then printf "changed during controlled fixture run\\n" > ../shared-sdk/VERSION; fi\nexec python3 scripts/bsp_lesson.py "$@"\n'

    if scenario == "prerequisites":
        sources["scripts/verify.sh"] = sources["scripts/verify.sh"].replace(b"build) mkdir -p build;", b"build) mkdir -p build; printf 'build invoked\\n' >> build/invocations.log;")
        sources["scripts/verify.sh"] = sources["scripts/verify.sh"].replace(b"run) result=", b"run) printf 'run invoked\\n' >> build/run-invocations.log; result=")
    provenance = encoded({"schemaVersion": 1, "sourceFiles": [
        {"path": name, "sha256": digest(data)} for name, data in sources.items()
    ]})
    files = {"workspace/" + name: data for name, data in sources.items()}
    files["provenance/provenance.lock.json"] = provenance
    manifest = {
        "schemaVersion": 3, "courseId": "lynx-local-acceptance", "version": "1.0.0",
        "title": "本地验收课程：Linux C 编译与证据（测试数据）",
        "boardProfile": "software-only-no-hardware",
        "provenanceSha256": digest(provenance),
        "workspace": {"remoteSubdir": "lesson", "packageSourceRoot": "workspace"},
        "labs": [],
    }
    steps = [
        ("source", "检查课程源码"), ("build", "编译失败、修复与重试"), ("run", "运行并核验真实输出")
    ]
    instructions = ("第二节先将 main.c 改成非法 C 语法，观察真实编译失败，修复后重新执行。\n"
                    "第三节应得到 LYNX local course: 42。\n")
    if scenario == "multimedia":
        manifest.update({"courseId": "lynx-local-media-acceptance",
                         "title": "本地验收课程：音频编码与证据（测试数据）"})
        steps = [("source", "生成合成音频样本"), ("build", "音频编码、失败修复与指标核验"),
                 ("run", "解码并验证无损往返")]
        instructions = ("需要 Python 3、FFmpeg 和 FFprobe；在 Ubuntu 工程运行，不采集麦克风、不操作板卡。\n\n"
                        "第一节生成 1 秒 440 Hz、16 kHz、单声道、16 位 PCM WAV，并记录内容摘要。\n"
                        "第二节编辑 settings.json：先把 codec 改为 lynx_missing_codec，观察真实编码失败；"
                        "改回 flac 后重试。FFprobe 必须读到 FLAC、16000 Hz、单声道、1 秒；"
                        "把 sampleRate 改为 8000 会因不符合课程目标而失败。\n"
                        "第三节解码为 PCM，实际字节和摘要须与源样本一致。查看 build/*-metrics.json。\n"
                        "样本、配置或编码文件变化时，必须重做受影响步骤；旧结果不能代替本次验证。\n")
    if scenario in ("bsp", "environment", "environment-execution"):
        manifest.update({"courseId": "lynx-local-bsp-acceptance",
                         "title": "本地验收课程：BSP 配置与 ARM 交叉编译（测试数据）"})
        steps = [("source", "检查 BSP 源码与交叉工具链"), ("build", "设备树与 ARM 配置编译、失败修复"),
                 ("run", "核验目标产物与设备树往返")]
        instructions = ("需要 Python 3、dtc、fdtget、arm-linux-gnueabihf-gcc 和 arm-linux-gnueabihf-objcopy。\n\n"
                        "本课程使用合成板级配置，内存基址 0x40000000、大小 64 MiB；不是真实 T113-Pro 设备树。\n"
                        "第一节读取源码摘要、实际工具版本与文件摘要。第二节先把 src/main.c 改成非法语法观察编译失败，"
                        "恢复后显式重试。实际生成 ARM32 ELF 配置对象和 DTB，并核对设备树内存与目标对象中的常量。\n"
                        "第三节重新读取产物、提取 ARM 配置，再把设备树反编译和重编译核对模型、兼容名与内存属性；"
                        "不会在 Ubuntu 执行 ARM 代码。查看 build/*-metrics.json 及对应产物摘要。\n"
                        "源码、工具或产物变化后必须重新构建。所有文件只用于本地学习，禁止直接部署或烧录到开发板。\n"
                        "这验证 BSP 配置的软件处理流程，不代表完整厂商 SDK、内核驱动、板上启动或实机验收。\n")
    for number, (name, title) in enumerate(steps, 1):
        document = (f"# {title}\n\n本课程是隔离的本地测试课程，不产生官网成绩或实机验收。\n\n"
                    "工程子目录为 lesson。源码由课程包 workspace 提供；准备到独立 SDK 工程后，"
                    "为 scripts/verify.sh 设置执行权限。\n\n"
                    + instructions).encode()
        doc_path = f"docs/{name}.md"
        files[doc_path] = document
        manifest["labs"].append({
            "stepId": name, "number": number, "path": name, "title": title,
            "learningObjective": title, "comprehensionCheck": "说明本节命令、退出码和观察结果的关系。",
            "studentAction": "在独立工程执行本节受控命令，记录真实结果。",
            "riskLevel": "software-safe", "expectedObservation": "命令成功退出并留下软件证据。",
            "verificationRules": [{"id": "exit-zero", "kind": "command",
                                   "command": f"./scripts/verify.sh {name}", "expectedExitCode": 0}],
            "requiredEvidence": [{"kind": "command", "source": "software"}],
            "reflectionPrompt": "解释本节结果及失败后的修复，不把软件通过等同于实机通过。",
            "document": {"path": doc_path, "sha256": digest(document),
                         "sourceUrl": "https://example.invalid/lynx-local-acceptance"},
        })

    if scenario in ("multimedia", "bsp", "environment", "environment-execution"):
        manifest["schemaVersion"] = 5
        for lab in manifest["labs"]:
            step = lab["stepId"]
            outputs = [{"artifactId": "metrics", "title": "本节指标", "path": f"build/{step}-metrics.json",
                        "mimeType": "application/json", "maxBytes": 65536}]
            if scenario == "multimedia":
                path, title, mime = {"source": ("samples/source.wav", "原始 WAV", "audio/wav"),
                                     "build": ("build/encoded.flac", "编码 FLAC", "audio/flac"),
                                     "run": ("build/decoded.wav", "解码 WAV", "audio/wav")}[step]
                outputs.append({"artifactId": "audio", "title": title, "path": path, "mimeType": mime, "maxBytes": 2097152})
            elif step != "source":
                outputs.extend([
                    {"artifactId": "dtb", "title": "设备树产物", "path": "build/board.dtb", "mimeType": "application/octet-stream", "maxBytes": 1048576},
                    {"artifactId": "arm", "title": "ARM 配置对象", "path": "build/config.o", "mimeType": "application/octet-stream", "maxBytes": 1048576}])
                if step == "run":
                    outputs.append({"artifactId": "dts", "title": "反编译设备树", "path": "build/roundtrip.dts", "mimeType": "text/plain", "maxBytes": 1048576})
            lab["artifactOutputs"] = outputs

    if scenario in ("theory", "mixed"):
        manifest["schemaVersion"] = 4
        manifest["courseId"] = "lynx-local-" + scenario + "-acceptance"
        manifest["title"] = "本地验收课程：" + ("理论阅读与复盘" if scenario == "theory" else "理论与 Linux 实践") + "（测试数据）"
        for lab in (manifest["labs"] if scenario == "theory" else manifest["labs"][:1]):
            lab.update({"activityKind": "reflection", "riskLevel": "reading-safe",
                        "verificationRules": [], "requiredEvidence": [],
                        "title": {"source": "阅读自述与真实证据", "build": "编译结果与设备运行", "run": "复盘与待验证事项"}[lab["stepId"]],
                        "learningObjective": "区分阅读自述、软件执行和实机观察。",
                        "comprehensionCheck": "为什么编译成功不能证明开发板运行正常？",
                        "studentAction": "阅读教材，用自己的话回答问题并完成复盘。",
                        "expectedObservation": "回答和复盘保存在本机；自述不自动评分。",
                        "reflectionPrompt": "总结三类证据的区别，记录仍不理解的问题。"})
            document = ("# 学习记录与证据\n\n这是合成测试课程，不产生官网成绩。\n\n"
                        "阅读自述表示学员已记录理解；编译退出码只说明本次编译结果；"
                        "实际设备运行需要对应目标和时间的观察。三者不能互相替代。\n\n"
                        "本活动无需 Ubuntu、SDK 或开发板。请回答问题，再完成复盘。"
                        "文字记录不自动判定答案正确，不调用 AI 评分。\n").encode()
            files[lab["document"]["path"]] = document
            lab["document"]["sha256"] = digest(document)
        if scenario == "theory":
            sources = {}
            files = {name: value for name, value in files.items() if not name.startswith("workspace/")}
            manifest.pop("workspace")
            provenance = encoded({"schemaVersion": 1, "sourceFiles": []})
            files["provenance/provenance.lock.json"] = provenance
            manifest["provenanceSha256"] = digest(provenance)

    if scenario == "optional":
        manifest.update({"schemaVersion": 6, "courseId": "lynx-local-optional-acceptance",
                         "title": "本地验收课程：必修与选修（测试数据）"})
        extra = []
        for step, title, reflection in [("optional-first", "选修：可跳过的准备活动", False),
                                         ("optional-failure", "选修：观察失败后决定是否继续", False),
                                         ("optional-reflection", "选修：拓展复盘", True)]:
            lab = copy.deepcopy(manifest["labs"][0])
            lab.update({"stepId": step, "path": step, "title": title, "required": False,
                        "learningObjective": "区分完成、失败和跳过；选修不阻塞必修汇总。"})
            document = ("# " + title + "\n\n这是合成选修测试活动。可以跳过，跳过不代表通过。\n"
                        "实践命令会真实退出 64，不修改工程文件；失败保留在学习记录中。\n").encode()
            doc_path = "docs/" + step + ".md"
            files[doc_path] = document
            lab["document"].update({"path": doc_path, "sha256": digest(document)})
            lab["verificationRules"][0]["command"] = "./scripts/verify.sh optional"
            if reflection:
                lab.update({"activityKind": "reflection", "riskLevel": "reading-safe",
                            "verificationRules": [], "requiredEvidence": []})
            extra.append(lab)
        manifest["labs"] = [extra[0], *manifest["labs"], *extra[1:]]
        for number, lab in enumerate(manifest["labs"], 1):
            lab["number"] = number
            lab.setdefault("required", True)

    if scenario == "hil-trust":
        manifest.update({"courseId": "lynx-local-hil-trust-acceptance",
                         "title": "本地验收课程：人工观察不能替代实机证据（合成测试）",
                         "boardProfile": "synthetic-hil-no-device"})
        manifest["labs"] = manifest["labs"][:1]
        image = b"SYNTHETIC TEST DATA - NEVER FLASH\n"
        files["fixtures/not-firmware.txt"] = image
        lab = manifest["labs"][0]
        lab.update({"riskLevel": "hardware-confirmation", "hilVerification": {
            "actionId": "synthetic-observation", "target": manifest["boardProfile"],
            "instructions": "只验证人工观察的可信等级，不连接、不采集、不操作任何开发板。",
            "requiredEvidenceKinds": ["wiring-observation", "flash-verification", "soak-observation"],
            "expectedObservations": ["人工声称通过仍应显示未验证"],
            "minimumDurationSeconds": 7200,
            "image": {"path": "fixtures/not-firmware.txt", "bytes": len(image),
                      "md5": hashlib.md5(image).hexdigest(), "sha256": digest(image)}}})
        document = ("# 人工观察与实机证据\n\n合成测试课程。软件步骤只检查文件存在。\n"
                    "填入匹配的摘要、PASS 和 7200 秒只是人工声称，不能证明设备运行。\n"
                    "本包没有可烧录固件，不得部署。测试不连接或操作任何开发板。\n").encode()
        files[lab["document"]["path"]] = document
        lab["document"]["sha256"] = digest(document)

    if scenario in ("environment", "environment-execution"):
        import subprocess
        manifest.update({"schemaVersion": 7, "courseId": "lynx-local-environment-acceptance",
                         "title": "本地验收课程：BSP 环境检查与差异恢复（合成测试）"})
        compiler = Path("/usr/bin/arm-linux-gnueabihf-gcc").resolve(strict=True)
        version = subprocess.check_output([str(compiler), "--version"], env={"PATH":"/usr/bin:/bin", "LC_ALL":"C"}, timeout=5).decode().splitlines()[0]
        checks = [
            {"id":"sdk", "title":"合成 SDK 版本文件", "kind":"file", "selector":"lesson/sdk/VERSION",
             "operator":"sha256", "expected":digest(sources["sdk/VERSION"]), "remediation":"恢复本测试包中的 SDK 版本文件，再重新检查；这不是厂商 SDK。"},
            {"id":"python", "title":"Python 3", "kind":"tool", "selector":"python3", "operator":"present", "expected":"",
             "remediation":"在 Ubuntu 准备系统 Python 3 后重新检查。"},
            {"id":"dtc", "title":"设备树编译器", "kind":"tool", "selector":"dtc", "operator":"present", "expected":"",
             "remediation":"在 Ubuntu 准备课程要求的 device-tree-compiler 后重新检查。"},
            {"id":"arm-version", "title":"ARM 交叉编译器版本", "kind":"tool", "selector":"arm-linux-gnueabihf-gcc", "operator":"eq", "expected":version,
             "remediation":"核对课程固定工具链版本；此合成包要求生成时主机的系统版本。"},
            {"id":"arm-digest", "title":"ARM 编译器程序摘要", "kind":"tool", "selector":"arm-linux-gnueabihf-gcc", "operator":"sha256", "expected":digest(compiler.read_bytes()),
             "remediation":"准备匹配本合成包的工具链程序后重新检查；相同版本文字不替代文件摘要。"},
        ]
        for lab in manifest["labs"]:
            lab["environmentChecks"] = copy.deepcopy(checks)

    if scenario == "environment-execution":
        manifest.update({"courseId":"lynx-local-environment-execution", "title":"本地验收课程：执行环境与报告重验（合成测试）"})
        manifest["labs"][0]["environmentChecks"][0]["selector"] = "shared-sdk/VERSION"
        manifest["labs"][1]["environmentChecks"] = [c for c in manifest["labs"][1]["environmentChecks"] if c["kind"] == "tool"]
        manifest["labs"][2]["environmentChecks"] = []
        external = destination / "prepared-sdk/shared-sdk"
        external.mkdir(parents=True, exist_ok=True)
        (external / "VERSION").write_bytes(sources["sdk/VERSION"])

    if scenario == "prerequisites":
        manifest.update({"schemaVersion":8,"courseId":"lynx-local-prerequisites", "title":"本地验收课程：独立学习与前置关系（合成测试）"})
        labs = {lab["stepId"]:lab for lab in manifest["labs"]}
        sdk_version = b"LYNX prerequisite SDK fixture 1\n"
        shared = destination / "prepared-sdk/shared-sdk/VERSION"
        shared.parent.mkdir(parents=True,exist_ok=True);shared.write_bytes(sdk_version)
        for name, title, required in [("intro","独立理论与前置关系",True),("extra","独立选修阅读",False)]:
            lab=copy.deepcopy(labs["source"])
            lab.update({"stepId":name,"path":name,"title":title,"activityKind":"reflection","riskLevel":"reading-safe","verificationRules":[],"requiredEvidence":[],"required":required,"studentAction":"阅读并记录理解；不执行工具。"})
            labs[name]=lab
        for name,lab in labs.items():
            lab["prerequisites"]={"run":["build"],"build":["source"]}.get(name,[])
            lab.setdefault("required",True)
            document=(f"# {lab['title']}\n\n本课程为签名合成测试数据，不产生官网成绩，不操作开发板。\n\n"
                      "目录展示顺序与前置关系独立。理论活动可先学；运行依赖构建，构建依赖源码实践。源码虽标为选修，但作为必修的明确前置时必须真正完成。\n\n"
                      "可跳转无依赖活动；跳过前置不会解锁后续。源码、工程绑定或前置声明 SDK 变化后，后续命令必须先停止，待前置重新验证。\n\n"
                      "源码工程位于 lesson；C 程序输出 LYNX local course: 42。shared-sdk/VERSION 是工程外合成依赖，不是厂商 SDK。\n").encode()
            doc_path=f"docs/{name}.md";files[doc_path]=document
            lab["document"]={"path":doc_path,"sha256":digest(document),"sourceUrl":"https://example.invalid/lynx-prerequisites"}
        labs["source"]["required"]=False
        labs["source"]["environmentChecks"]=[{"id":"sdk","title":"前置合成 SDK","kind":"file","selector":"shared-sdk/VERSION","operator":"sha256","expected":digest(sdk_version),"remediation":"恢复 shared-sdk/VERSION 的测试基线后重新核验前置活动。"}]
        manifest["labs"]=[labs[name] for name in ["run","intro","source","build","extra"]]
        for number,lab in enumerate(manifest["labs"],1):lab["number"]=number

    def package(name, spec, entries, signing_key=key):
        buffer = io.BytesIO()
        with zipfile.ZipFile(buffer, "w", zipfile.ZIP_DEFLATED) as archive:
            for file_name, data in {**entries, "manifest.json": encoded(spec)}.items():
                archive.writestr(file_name, data)
        payload = buffer.getvalue()
        raw_digest = hashlib.sha256(payload).digest()
        envelope = {
            "format": "lynx-course-v1", "algorithm": "Ed25519-SHA256",
            "publicKey": base64.b64encode(signing_key.public_key().public_bytes(
                serialization.Encoding.Raw, serialization.PublicFormat.Raw)).decode(),
            "sha256": raw_digest.hex(),
            "signature": base64.b64encode(signing_key.sign(raw_digest)).decode(),
            "payload": base64.b64encode(payload).decode(),
        }
        (destination / (name + ".lynx-course")).write_bytes(encoded(envelope))
        return envelope

    good = package("course", manifest, files)
    if scenario == "prerequisites":
        for name, edges in [("cycle",["run"]),("missing",["absent"]),("duplicate",["source","source"]),("self",["build"])]:
            bad_graph=copy.deepcopy(manifest);bad_graph["labs"][3]["prerequisites"]=edges
            package("bad-prerequisites-"+name,bad_graph,files)
        old_graph=copy.deepcopy(manifest);old_graph["schemaVersion"]=7
        package("bad-prerequisites-legacy",old_graph,files)
    corrupted = {**good, "signature": base64.b64encode(bytes(64)).decode()}
    (destination / "bad-signature.lynx-course").write_bytes(encoded(corrupted))
    package("bad-path", manifest, {**files, "../escaped.txt": b"must not escape"})
    package("bad-provenance", manifest, {**files, ("workspace/" + next(iter(sources)) if sources else "provenance/provenance.lock.json"): b"modified"})
    bad_command = copy.deepcopy(manifest)
    bad_command["labs"][0]["verificationRules"] = [{"id": "bad", "kind": "command", "command": "./scripts/verify.sh; true", "expectedExitCode": 0}]
    package("bad-command", bad_command, files)
    practical_index = next((i for i, lab in enumerate(manifest["labs"])
                            if lab.get("activityKind", "practice") == "practice"), None)
    if practical_index is not None:
        for name, command in [
            ("bad-substitution", "./scripts/verify.sh $(printf source)"),
            ("bad-quoted-traversal", "./scripts/.'./'outside"),
            ("bad-glob", "./scripts/*.sh source"),
        ]:
            bad_literal = copy.deepcopy(manifest)
            bad_literal["labs"][practical_index]["verificationRules"][0]["command"] = command
            package(name, bad_literal, files)
    if scenario in ("multimedia", "bsp", "environment", "environment-execution"):
        for name, field, value in [("bad-artifact-path", "path", "../private"),
                                   ("bad-artifact-type", "mimeType", "text/html"),
                                   ("bad-artifact-budget", "maxBytes", 33554433)]:
            bad_artifact = copy.deepcopy(manifest)
            bad_artifact["labs"][0]["artifactOutputs"][0][field] = value
            package(name, bad_artifact, files)
        old_schema_artifact = copy.deepcopy(manifest)
        old_schema_artifact["schemaVersion"] = 4
        package("bad-artifact-old-schema", old_schema_artifact, files)
    if scenario == "optional":
        bad_optional = copy.deepcopy(manifest)
        bad_optional["schemaVersion"] = 5
        package("bad-optional-old-schema", bad_optional, files)
    next_version = {**manifest, "version": "1.0.1"}
    package("bad-signer", next_version, files, Ed25519PrivateKey.generate())
    for name, data in sources.items():
        target = destination / "prepared-sdk" / "lesson" / name
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(data)
        if name.endswith(".sh"):
            target.chmod(0o755)
    (destination / "fixture.json").write_bytes(encoded({
        "synthetic": True, "officialGrades": False, "hardwareActions": False,
        "scenario": scenario,
        "courseId": manifest["courseId"], "version": manifest["version"],
        "packageSha256": digest(encoded(good)), "payloadSha256": good["sha256"],
        "signerFingerprint": digest(base64.b64decode(good["publicKey"])),
    }))
    print(destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("destination", type=Path, help="A new isolated output directory")
    parser.add_argument("--scenario", choices=("c", "multimedia", "theory", "mixed", "bsp", "optional", "hil-trust", "environment", "environment-execution", "prerequisites"), default="c")
    args = parser.parse_args()
    generate(args.destination.resolve(), scenario=args.scenario)
