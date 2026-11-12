export const formatPlaybackTime = (seconds: number) => {
  const total = Number.isFinite(seconds) && seconds > 0 ? Math.floor(seconds) : 0;
  const minutes = Math.floor(total / 60);
  const remainder = (total % 60).toString().padStart(2, '0');

  return `${minutes}:${remainder}`;
};
