const SILENT_WAV =
  'data:audio/wav;base64,UklGRigAAABXQVZFZm10IBIAAAABAAEARKwAAIhYAQACABAAAABkYXRhAgAAAAEA';

let warmupPromise: Promise<void> | null = null;

export const warmupPlayback = () => {
  if (warmupPromise) {
    return warmupPromise;
  }

  warmupPromise = new Promise((resolve) => {
    const audio = new Audio();
    audio.volume = 0;
    audio.preload = 'auto';
    audio.src = SILENT_WAV;

    const finish = () => {
      audio.pause();
      audio.removeAttribute('src');
      audio.load();
      resolve();
    };

    audio.addEventListener('canplay', finish, { once: true });
    audio.addEventListener('error', finish, { once: true });

    void audio.play().catch(finish);
  });

  return warmupPromise;
};
