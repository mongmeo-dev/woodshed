# Audio fixtures

1 second, 440 Hz sine at half scale, 44.1 kHz, generated with FFmpeg:

```sh
ffmpeg -f lavfi -i "aevalsrc=0.5*sin(2*PI*440*t):s=44100:d=1" -ac 1 -c:a pcm_s16le tone.wav
ffmpeg -i tone.wav -c:a libmp3lame -b:a 64k tone-440.mp3
ffmpeg -i tone.wav -c:a flac tone-440.flac
ffmpeg -i tone.wav -c:a aac -b:a 64k tone-440.m4a
ffmpeg -i tone.wav -c:a vorbis -strict -2 -ac 2 tone-440.ogg
```

The WAV case is written by the test itself.
