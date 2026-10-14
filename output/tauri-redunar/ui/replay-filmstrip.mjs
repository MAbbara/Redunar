const FRAME_COUNT = 8;
const FRAME_WIDTH = 160;
const FRAME_HEIGHT = 90;

function waitForMedia(target, event, signal, begin, timeout = 2500) {
  return new Promise((resolve, reject) => {
    let timer;
    const finish = error => {
      clearTimeout(timer);
      target.removeEventListener(event, ready);
      target.removeEventListener('error', failed);
      signal.removeEventListener('abort', aborted);
      error ? reject(error) : resolve();
    };
    const ready = () => finish();
    const failed = () => finish(new Error('Preview unavailable'));
    const aborted = () => finish(new Error('Preview cancelled'));
    if (signal.aborted) return aborted();
    target.addEventListener(event, ready, {once: true});
    target.addEventListener('error', failed, {once: true});
    signal.addEventListener('abort', aborted, {once: true});
    if (timeout) timer = setTimeout(failed, timeout);
    try { begin?.(); } catch (error) { finish(error); }
  });
}

// Reuse the selected, prepared stream. One muted decoder samples eight frames;
// it never seeks the player, starts FFmpeg, or creates another native token.
// Pause sampling during playback/hidden windows, and release it on selection.
export function renderReplayFilmstrip(player, canvas, status) {
  const controller = new AbortController();
  const {signal} = controller;
  const preview = document.createElement('video');
  preview.muted = true;
  // Metadata-only preloading can stall WebKitGTK on captured H.264/AAC clips.
  // Sampling needs decoded frames, so use the same normal load as the player.
  preview.preload = 'auto';
  preview.crossOrigin = 'anonymous';
  // WebKitGTK can expose loadeddata/seeked on a detached video while canvas
  // copies remain transparent. Keep the bounded sampler attached, visually
  // hidden and outside input navigation, until its decoder is released.
  preview.setAttribute('aria-hidden', 'true');
  preview.tabIndex = -1;
  Object.assign(preview.style, {position: 'fixed', width: '1px', height: '1px', opacity: '0', pointerEvents: 'none'});
  document.body.append(preview);
  canvas.width = FRAME_COUNT * FRAME_WIDTH;
  canvas.height = FRAME_HEIGHT;
  const context = canvas.getContext('2d');
  const release = () => {
    preview.removeAttribute('src');
    preview.load();
    preview.remove();
  };
  const waitUntilIdle = async () => {
    while (!signal.aborted && (!player.paused || document.hidden)) {
      if (!player.paused) await waitForMedia(player, 'pause', signal, null, 0);
      if (document.hidden) await waitForMedia(document, 'visibilitychange', signal, null, 0);
    }
    if (signal.aborted) throw new Error('Preview cancelled');
  };
  async function render() {
    let completed = 0;
    try {
      await waitUntilIdle();
      await waitForMedia(preview, 'loadeddata', signal, () => { preview.src = player.currentSrc || player.src; });
      for (let index = 0; index < FRAME_COUNT; index++) {
        await waitUntilIdle();
        const seconds = index / (FRAME_COUNT - 1) * Math.max(0, player.duration - .1);
        if (Math.abs(preview.currentTime - seconds) > .01) {
          await waitForMedia(preview, 'seeked', signal, () => { preview.currentTime = seconds; });
        }
        // WebKit can fire seeked before its decoded frame is painted. Give
        // that frame a rendering turn before copying it into the filmstrip.
        await new Promise(resolve => setTimeout(resolve, 32));
        if (signal.aborted) return;
        const scale = Math.max(FRAME_WIDTH / preview.videoWidth, FRAME_HEIGHT / preview.videoHeight);
        const width = FRAME_WIDTH / scale, height = FRAME_HEIGHT / scale;
        context.drawImage(preview, (preview.videoWidth - width) / 2, (preview.videoHeight - height) / 2,
          width, height, index * FRAME_WIDTH, 0, FRAME_WIDTH, FRAME_HEIGHT);
        completed++;
        canvas.dataset.frames = String(completed);
      }
      status.hidden = true;
    } catch {
      if (!signal.aborted) status.textContent = completed ? 'Some frame previews unavailable' : 'Frame previews unavailable · trimming still works';
    } finally {
      release();
    }
  }
  void render();
  return () => { controller.abort(); release(); };
}
