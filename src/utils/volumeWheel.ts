export function wheelVolumeChange(event: Pick<WheelEvent, 'deltaMode' | 'deltaX' | 'deltaY'>) {
  const { deltaMode, deltaX, deltaY } = event;
  if (!Number.isFinite(deltaY) || deltaY === 0 || Math.abs(deltaX) > Math.abs(deltaY)) return 0;
  const scale = deltaMode === WheelEvent.DOM_DELTA_LINE ? 0.025
    : deltaMode === WheelEvent.DOM_DELTA_PAGE ? 0.1 : 0.001;
  return -Math.sign(deltaY) * Math.min(0.1, Math.abs(deltaY) * scale);
}
