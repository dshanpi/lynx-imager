import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import ts from 'typescript';

const code = await readFile(
  new URL('../../src/Components/Imager/imageReviewModel.ts', import.meta.url),
  'utf8'
);
const { outputText } = ts.transpileModule(code, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
});
const model = await import(
  'data:text/javascript;base64,' + Buffer.from(outputText).toString('base64')
);
const hash = 'ab'.repeat(32);
const raw = {
  sizeBytes: 614400,
  sha256: hash,
  egonAtZero: true,
  splAt8192: false,
  entries: [],
  containerVersion: null,
};
const entry = (mainType, subtype, sizeBytes = 1) => ({
  mainType,
  subtype,
  sizeBytes,
  name: 'entry',
  offset: 1024,
});
const ram = [entry('FES', 'FES_1-0000000000'), entry('12345678', 'UBOOT_0000000000')];
const pkg = (entries) => ({ ...raw, egonAtZero: false, containerVersion: 3, entries });

test('routes reflect board storage; IMAGEWTY alone cannot establish its role', () => {
  assert.equal(model.awMainline('SPI NOR'), 'mainline_spinor');
  assert.equal(model.awMainline('SPI NAND'), 'mainline_nand');
  assert.equal(model.awMainline('eMMC'), 'mainline_emmc');
  assert.equal(model.awMainline('SD'), undefined);
  assert.ok(model.sourceProblem('vendor', pkg(ram)));
  assert.ok(model.sourceProblem('mainline_nand', pkg(ram)));
  assert.ok(model.sourceProblem('mainline_spinor', pkg(ram)));
  const layout = entry('12345678', '1234567890___MBR');
  assert.equal(model.sourceProblem('vendor', pkg([...ram, layout])), '');
  assert.ok(model.sourceProblem('mainline_nand', pkg([...ram, layout])));
  assert.equal(
    model.sourceProblem(
      'mainline_nand',
      pkg([...ram, layout, entry('12345678', 'BOOT0_0000000000')])
    ),
    ''
  );
  assert.equal(
    model.ramComponents(pkg([entry('wrong', 'FES_1-0000000000'), ram[1]]))[0],
    undefined
  );
});

test('eMMC and NOR markers and usable image length are checked independently', () => {
  assert.equal(model.sourceProblem('mainline_spinor', raw), '');
  assert.ok(model.sourceProblem('mainline_emmc', raw));
  assert.ok(model.sourceProblem('mainline_spinor', { ...raw, sizeBytes: 524288 }));
  assert.equal(
    model.sourceProblem('mainline_emmc', { ...raw, egonAtZero: false, splAt8192: true }),
    ''
  );
});

test('reference validation distinguishes file digest, model and application boundaries', () => {
  const ref = {
    sha256: hash,
    sizeBytes: 614400,
    applicationOffset: 524288,
    applicationBytes: 89584,
    boardModel: 'yuzukineko',
    fileName: null,
  };
  const check = (fields) =>
    model.referenceProblem({ ...ref, ...fields }, raw, 'mainline_spinor', 'F101');
  assert.equal(check({}), '');
  assert.match(check({ sha256: 'de'.repeat(32) }), /SHA-256/);
  assert.match(check({ sizeBytes: 600000 }), /大小/);
  assert.match(check({ boardModel: 'T113-Pro' }), /板型/);
  assert.match(check({ applicationOffset: 0 }), /偏移/);
  assert.match(check({ applicationBytes: 999999 }), /超出/);
});
