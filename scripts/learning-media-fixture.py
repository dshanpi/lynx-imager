#!/usr/bin/env python3
"""Synthetic audio lesson source, embedded in signed test courses only."""
import hashlib
import json
import math
from pathlib import Path
import re
import struct
import subprocess
import sys
import wave


def sha(data):
    return hashlib.sha256(data).hexdigest()


def command(args):
    result = subprocess.run(args, capture_output=True, timeout=20, check=False)
    if result.returncode:
        raise ValueError(result.stderr.decode(errors='replace')[-4096:])
    return result.stdout.decode()


def wav(path):
    if path.stat().st_size > 2 * 1024 * 1024:
        raise ValueError('Audio exceeds lesson bound')
    with wave.open(str(path), 'rb') as audio:
        facts = (audio.getnchannels(), audio.getsampwidth(), audio.getframerate(), audio.getnframes())
        pcm = audio.readframes(audio.getnframes())
    if facts != (1, 2, 16000, 16000) or len(pcm) != 32000:
        raise ValueError('Expected 1 second, mono, 16-bit PCM at 16000 Hz')
    return pcm


def save(name, facts):
    Path('build').mkdir(exist_ok=True)
    target = Path('build') / (name + '-metrics.json')
    temp = target.with_suffix('.tmp')
    text = json.dumps(facts, sort_keys=True)
    temp.write_text(text + '\n')
    temp.replace(target)
    print(text)


def inspect():
    result = json.loads(command(['ffprobe', '-v', 'error', '-show_entries',
                                 'stream=codec_name,sample_rate,channels:format=duration',
                                 '-of', 'json', 'build/encoded.flac']))
    streams = result['streams']
    if len(streams) != 1:
        raise ValueError('Expected one audio stream')
    stream = streams[0]
    facts = {'codec': stream['codec_name'], 'sampleRate': int(stream['sample_rate']),
             'channels': stream['channels'], 'durationSeconds': float(result['format']['duration'])}
    if (facts['codec'], facts['sampleRate'], facts['channels']) != ('flac', 16000, 1):
        raise ValueError('Expected FLAC, 16000 Hz, mono')
    if not math.isfinite(facts['durationSeconds']) or abs(facts['durationSeconds'] - 1) > 0.001:
        raise ValueError('Expected duration 1 second')
    return facts


def main(step):
    if step not in ('source', 'build', 'run'):
        raise ValueError('Unknown lesson action')
    # Old successful metrics cannot survive a failed new action.
    for name in {'source': ('source', 'build', 'run'), 'build': ('build', 'run'), 'run': ('run',)}[step]:
        (Path('build') / (name + '-metrics.json')).unlink(missing_ok=True)
    if step == 'source':
        Path('samples').mkdir(exist_ok=True)
        pcm = b''.join(struct.pack('<h', round(12000 * math.sin(2 * math.pi * 440 * i / 16000)))
                       for i in range(16000))
        with wave.open('samples/source.wav', 'wb') as audio:
            audio.setparams((1, 2, 16000, 0, 'NONE', 'not compressed'))
            audio.writeframes(pcm)
        pcm = wav(Path('samples/source.wav'))
        save('source', {'synthetic': True, 'frequencyHz': 440, 'frames': 16000,
                        'sampleRate': 16000, 'channels': 1, 'pcmSha256': sha(pcm)})
        return
    source = wav(Path('samples/source.wav'))
    source_facts = json.loads(Path('build/source-metrics.json').read_text())
    if sha(source) != source_facts['pcmSha256']:
        raise ValueError('Source changed; regenerate and repeat encoding')
    settings = Path('settings.json').read_bytes()
    config = json.loads(settings)
    if step == 'build':
        codec = config['codec']
        rate, channels = config['sampleRate'], config['channels']
        if not isinstance(codec, str) or not re.fullmatch(r'[a-zA-Z0-9_]{1,64}', codec):
            raise ValueError('Invalid encoder identifier')
        if type(rate) is not int or not 8000 <= rate <= 48000 or type(channels) is not int or channels not in (1, 2):
            raise ValueError('Invalid bounded audio settings')
        command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-y', '-i',
                 'samples/source.wav', '-c:a', codec, '-ar', str(rate), '-ac', str(channels),
                 'build/encoded.flac'])
        facts = inspect()
        facts.update({'sourcePcmSha256': sha(source), 'settingsSha256': sha(settings),
                      'encodedSha256': sha(Path('build/encoded.flac').read_bytes()),
                      'ffmpegVersion': command(['ffmpeg', '-version']).splitlines()[0],
                      'ffprobeVersion': command(['ffprobe', '-version']).splitlines()[0]})
        save('build', facts)
        return
    built = json.loads(Path('build/build-metrics.json').read_text())
    if built['settingsSha256'] != sha(settings) or built['sourcePcmSha256'] != sha(source):
        raise ValueError('Encoding inputs changed; repeat encoding')
    if built['encodedSha256'] != sha(Path('build/encoded.flac').read_bytes()):
        raise ValueError('Encoded media changed; repeat encoding')
    facts = inspect()
    command(['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'error', '-y', '-i',
             'build/encoded.flac', '-c:a', 'pcm_s16le', 'build/decoded.wav'])
    decoded = wav(Path('build/decoded.wav'))
    if decoded != source:
        raise ValueError('Decoded PCM differs from source')
    facts.update({'sourcePcmSha256': sha(source), 'decodedPcmSha256': sha(decoded), 'lossless': True})
    save('run', facts)


if __name__ == '__main__':
    try:
        if len(sys.argv) != 2:
            raise ValueError('Exactly one lesson action is required')
        main(sys.argv[1])
    except (ValueError, KeyError, TypeError, OSError, wave.Error, subprocess.TimeoutExpired) as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
