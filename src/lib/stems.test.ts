import {
  formatTime,
  importErrorCode,
  isAudioFile,
  positionAt,
  STEM_NAMES,
  type StemStatus,
  separationFromStems,
  stemsReady,
  youtubeVideoId,
} from "./stems";

test("YouTube links resolve to their video id", () => {
  for (const link of [
    "https://www.youtube.com/watch?v=jNQXAC9IVRw",
    "https://youtube.com/watch?feature=share&v=jNQXAC9IVRw&t=10",
    "http://m.youtube.com/watch?v=jNQXAC9IVRw",
    "https://music.youtube.com/watch?v=jNQXAC9IVRw&list=RD",
    "youtube.com/watch?v=jNQXAC9IVRw",
    "https://youtu.be/jNQXAC9IVRw?si=abc",
    "  https://www.youtube.com/shorts/jNQXAC9IVRw  ",
    "https://www.youtube.com/live/jNQXAC9IVRw",
    "https://www.youtube-nocookie.com/embed/jNQXAC9IVRw",
    "HTTPS://WWW.YOUTUBE.COM/watch?v=jNQXAC9IVRw#t=5",
  ]) {
    expect(youtubeVideoId(link)).toBe("jNQXAC9IVRw");
  }
  for (const link of [
    "",
    "hello",
    "https://vimeo.com/123456",
    "https://www.youtube.com/",
    "https://www.youtube.com/watch?v=short",
    "https://www.youtube.com/playlist?list=PL123",
    "https://notyoutube.com/watch?v=jNQXAC9IVRw",
    "ftp://youtube.com/watch?v=jNQXAC9IVRw",
  ]) {
    expect(youtubeVideoId(link)).toBeNull();
  }
});

test("position extrapolates at the playback speed from the heard time", () => {
  const sample = { positionSecs: 10, playing: true };
  expect(positionAt(sample, 1_000, 3_000, 1, null, 100)).toBe(12);
  expect(positionAt(sample, 1_000, 3_000, 0.5, null, 100)).toBe(11);
  // Not heard yet: still before the sampled position.
  expect(positionAt(sample, 1_000, 500, 1, null, 100)).toBe(9.5);
  expect(positionAt({ ...sample, playing: false }, 1_000, 9_000, 1, null, 100)).toBe(10);
  expect(positionAt(sample, 1_000, 999_000, 1, null, 100)).toBe(100);
});

test("position wraps inside the loop the way the player does", () => {
  const loop = { start: 10, end: 14 };
  expect(positionAt({ positionSecs: 13, playing: true }, 0, 2_000, 1, loop, 100)).toBe(11);
  // Just wrapped to the start, but the end of the lap is still being heard.
  expect(positionAt({ positionSecs: 10.5, playing: true }, 1_000, 0, 1, loop, 100)).toBe(13.5);
  // Approaching the loop from before it.
  expect(positionAt({ positionSecs: 8, playing: true }, 0, 1_000, 1, loop, 100)).toBe(9);
  expect(positionAt({ positionSecs: 8, playing: true }, 0, 7_000, 1, loop, 100)).toBe(11);
  // Past the loop end it plays on.
  expect(positionAt({ positionSecs: 20, playing: true }, 0, 1_000, 1, loop, 100)).toBe(21);
});

test("times show as m:ss", () => {
  expect(formatTime(0)).toBe("0:00");
  expect(formatTime(61.9)).toBe("1:01");
  expect(formatTime(3_725)).toBe("1:02:05");
});

test("import errors and file types are recognised", () => {
  expect(importErrorCode({ code: "network" })).toBe("network");
  expect(importErrorCode({ code: "whatever" })).toBe("failed");
  expect(importErrorCode("boom")).toBe("failed");
  expect(isAudioFile("/a/b/Song.MP3")).toBe(true);
  expect(isAudioFile("/a/b/song.m4a")).toBe(true);
  expect(isAudioFile("/a/b/notes.txt")).toBe(false);
});

test("a song is separated only when all its stems are ready", () => {
  const song = (status: StemStatus, last: StemStatus = status) => ({
    id: "s1",
    stems: STEM_NAMES.map((name, i) => ({ name, status: i === 5 ? last : status })),
  });
  expect(stemsReady({ stems: [] })).toBe(false);
  expect(stemsReady(song("ready"))).toBe(true);
  expect(stemsReady(song("ready", "running"))).toBe(false);
  expect(separationFromStems({ id: "s1", stems: [] })).toBeNull();
  expect(separationFromStems(song("ready"))).toBeNull();
  expect(separationFromStems(song("failed"))).toEqual({
    songId: "s1",
    state: "failed",
    progress: null,
  });
  for (const status of ["pending", "running"] as const) {
    expect(separationFromStems(song(status))).toEqual({
      songId: "s1",
      state: "running",
      progress: null,
    });
  }
});
