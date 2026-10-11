import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import sharp from 'sharp'
import { fileURLToPath } from 'node:url'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const fixturesDir = path.resolve(__dirname, '../fixtures/augment-ocr')
const manifestPath = path.join(fixturesDir, 'manifest.json')
const originalCwd = process.cwd()
const originalHomedir = os.homedir
const originalOcrLocale = process.env.ARAMGG_OCR_LOCALE
let testRoot = null
let shutdownImageAnalyzer = null
const benchmarkRecords = []

async function writeJson(filePath, payload) {
    await fs.mkdir(path.dirname(filePath), { recursive: true })
    await fs.writeFile(filePath, JSON.stringify(payload), 'utf8')
}

async function seedFixtureData(manifest) {
    const dataRoot = path.join(testRoot, '.aramgg_client', 'data')
    const dataVersion = 'ocr-fixtures-v1'
    const augments = [...new Map(
        manifest.flatMap(sample => sample.expectedIds.map((id, index) => [
            Number(id),
            {
                id: Number(id),
                name: sample.expectedNames[index],
                rarity: 'kGold',
                iconPath: '',
            },
        ]))
    ).values()]

    for (const locale of ['zh-CN', 'en-US', 'zh-TW']) {
        const pointerName = locale === 'zh-CN' ? 'current.json' : `current.${locale}.json`
        const versionDir = locale === 'zh-CN'
            ? path.join(dataRoot, 'versions', dataVersion)
            : path.join(dataRoot, 'versions', locale, dataVersion)

        await writeJson(path.join(dataRoot, pointerName), {
            schemaVersion: 3,
            locale,
            dataVersion,
        })
        await writeJson(path.join(versionDir, 'manifest.json'), {
            locale,
            dataVersion,
            files: [{ path: 'augments.json' }],
        })
        await writeJson(path.join(versionDir, 'augments.json'), { augments })
    }
}

function idsOf(augments = []) {
    return augments.map(augment => Number(augment.id))
}

function namesOf(augments = []) {
    return augments.map(augment => String(augment.name))
}

function enginesOf(slotDiagnostics = []) {
    return [...new Set(slotDiagnostics.map(diagnostic => diagnostic.ocrEngine).filter(Boolean))]
}

async function assertFixtureResult(analyzeScreenshot, input, sample, variant) {
    const startedAt = performance.now()
    const cpuStarted = process.cpuUsage()
    const result = await analyzeScreenshot(input)
    const cpu = process.cpuUsage(cpuStarted)
    benchmarkRecords.push({ file: sample.file, variant, wallMs: performance.now() - startedAt, cpuMs: (cpu.user + cpu.system) / 1000 })

    assert.equal(result.success, true, `${sample.file} (${variant}): analysis should succeed`)
    assert.equal(
        result.analysis.cardCount,
        sample.expectedCardCount,
        `${sample.file} (${variant}): cardCount should match fixture expectation`
    )
    assert.deepEqual(
        idsOf(result.analysis.augments),
        sample.expectedIds,
        `${sample.file} (${variant}): augment ids should remain stable`
    )
    assert.deepEqual(
        namesOf(result.analysis.augments),
        sample.expectedNames,
        `${sample.file} (${variant}): augment names should remain stable`
    )

    const engines = enginesOf(result.analysis.slotDiagnostics)
    if (variant === '1280x720' || engines.length > 0) {
        assert.deepEqual(engines, ['paddleocr'], `${sample.file} (${variant}): OCR engine should be PaddleOCR only`)
    }

    console.log(JSON.stringify({
        file: sample.file,
        variant,
        cardCount: result.analysis.cardCount,
        ids: idsOf(result.analysis.augments),
        names: namesOf(result.analysis.augments),
        durationMs: result.metadata.analysisDurationMs,
    }))
}

async function assertGateResult(analyzeScreenshotGate, input, sample, variant) {
    const result = await analyzeScreenshotGate(input)

    assert.equal(result.success, true, `${sample.file} (${variant}): gate analysis should succeed`)
    assert.equal(
        result.likely,
        true,
        `${sample.file} (${variant}): title activity gate should pass for a card frame`
    )
    assert.equal(
        result.rerollVisible,
        true,
        `${sample.file} (${variant}): reroll button gate should pass for a card frame`
    )

    console.log(JSON.stringify({
        file: sample.file,
        variant,
        likely: result.likely,
        rerollVisible: result.rerollVisible,
        durationMs: result.durationMs,
    }))
}

async function main() {
    const manifest = JSON.parse(await fs.readFile(manifestPath, 'utf8'))
    testRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'aramgg-augment-ocr-'))
    // Keep app data isolated without redirecting the native GPU driver's user profile.
    os.homedir = () => testRoot
    process.env.ARAMGG_OCR_LOCALE = 'zh-CN'
    process.chdir(testRoot)
    await seedFixtureData(manifest)

    const imageAnalyzer = await import('../../src/main/image-analyzer.ts')
    const analyzeScreenshot = imageAnalyzer.analyzeScreenshot
    const analyzeScreenshotGate = imageAnalyzer.analyzeScreenshotGate
    shutdownImageAnalyzer = imageAnalyzer.shutdownImageAnalyzer

    const rounds = Math.max(1, Math.min(30, Number(process.env.ARAMGG_OCR_BENCHMARK_ROUNDS) || 1))
    for (let round = 0; round < rounds; round++) {
      for (const sample of manifest) {
        const imagePath = path.join(fixturesDir, sample.file)
        await assertFixtureResult(analyzeScreenshot, imagePath, sample, '1280x720')
        if (sample.expectedCardCount > 0) {
            const gateBuffer = await sharp(imagePath)
                .resize(640, 360, { fit: 'fill' })
                .png()
                .toBuffer()
            await assertGateResult(analyzeScreenshotGate, gateBuffer, sample, '640x360')
        }
        const automaticCaptureBuffer = await sharp(imagePath)
            .resize(1024, 576, { fit: 'fill' })
            .png()
            .toBuffer()
        await assertFixtureResult(analyzeScreenshot, automaticCaptureBuffer, sample, '1024x576')
        if (rounds === 1) {
            const { data, info } = await sharp(automaticCaptureBuffer).ensureAlpha().raw().toBuffer({ resolveWithObject: true })
            const rawFrame = { buffer: data, width: info.width, height: info.height, channels: 4 }
            await assertFixtureResult(analyzeScreenshot, rawFrame, sample, 'raw1024x576')
            if (sample.expectedCardCount === 3) {
                // Changing the background must not force unchanged titles through OCR again.
                await analyzeScreenshot(rawFrame)
                const hitsBefore = imageAnalyzer.getTitleCacheStatus().hits
                const changedBackground = { ...rawFrame, buffer: Buffer.from(data) }
                changedBackground.buffer[0] ^= 1
                await assertFixtureResult(analyzeScreenshot, changedBackground, sample, 'title-cache-background-change')
                assert.ok(imageAnalyzer.getTitleCacheStatus().hits > hitsBefore, 'unchanged titles must reuse OCR after background changes')
            }
            if (sample.expectedCardCount > 0) await assertGateResult(analyzeScreenshotGate, rawFrame, sample, 'raw-gate')
        }
      }
    }
    if (process.env.ARAMGG_OCR_BENCHMARK_OUTPUT) {
        await fs.writeFile(path.resolve(originalCwd, process.env.ARAMGG_OCR_BENCHMARK_OUTPUT), JSON.stringify({ rounds, records: benchmarkRecords }, null, 2))
    }
}

main()
    .catch(error => {
        console.error(error)
        process.exitCode = 1
    })
    .finally(async () => {
        if (shutdownImageAnalyzer) {
            await shutdownImageAnalyzer()
        }
        process.chdir(originalCwd)
        os.homedir = originalHomedir
        if (originalOcrLocale == null) {
            delete process.env.ARAMGG_OCR_LOCALE
        } else {
            process.env.ARAMGG_OCR_LOCALE = originalOcrLocale
        }
        if (testRoot) {
            await fs.rm(testRoot, { recursive: true, force: true })
        }
    })
