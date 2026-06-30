import { useAtom, useAtomValue, useSetAtom } from 'jotai';
import {
  currentTimeAtom,
  currentTrackAtom,
  durationAtom,
  playingAtom,
  playNextAtom,
  playPreviousAtom,
  seekingAtom,
  volumeAtom,
} from '@/atoms/music';
import { Next } from '@/components/Icons/Next';
import { Pause } from '@/components/Icons/Pause';
import { Play } from '@/components/Icons/Play';
import { Previous } from '@/components/Icons/Previous';
import { Repeat } from '@/components/Icons/Repeat';
import { Shuffle } from '@/components/Icons/Shuffle';
import { SeekBar } from '@/components/SeekBar';
import { SliderFrame } from '@/components/Slider';
import { TrackPlaying } from '@/components/TrackPlaying';
import { VolumeBar } from '@/components/VolumeBar';

const formatTime = (seconds: number) => {
  if (!Number.isFinite(seconds) || seconds < 0) {
    return '0:00';
  }

  const total = Math.floor(seconds);
  const minutes = Math.floor(total / 60);
  const remainder = total % 60;

  return `${minutes}:${remainder.toString().padStart(2, '0')}`;
};

export const PlayerControls = () => {
  const currentTrack = useAtomValue(currentTrackAtom);
  const [playing, setPlaying] = useAtom(playingAtom);
  const [time, setTime] = useAtom(currentTimeAtom);
  const [, setSeeking] = useAtom(seekingAtom);
  const [volume, setVolume] = useAtom(volumeAtom);
  const playNext = useSetAtom(playNextAtom);
  const playPrevious = useSetAtom(playPreviousAtom);
  const duration = useAtomValue(durationAtom);
  const sliderMax = Math.max(duration, 0.1);
  const sliderValue = Math.min(time, sliderMax);

  return (
    <div class='relative h-[110px] flex items-center justify-center p-4 flex-col gap-1 bg-[#061232]'>
      <TrackPlaying track={currentTrack} />
      <div className='flex flex-col min-w-100 justify-center items-center'>
        <div className='flex items-center *:cursor-pointer gap-2'>
          <Shuffle />
          <Previous onPointerDown={() => playPrevious()} />
          {!playing ? (
            <Play size={50} onPointerDown={() => setPlaying(true)} />
          ) : (
            <Pause size={50} onPointerDown={() => setPlaying(false)} />
          )}
          <Next onPointerDown={() => playNext()} />
          <Repeat />
        </div>
        <SliderFrame>
          <span className='w-10 text-right text-xs text-white/60 tabular-nums'>{formatTime(time)}</span>
          <SeekBar
            className='max-w-[468px] flex-1'
            value={sliderValue}
            min={0}
            max={sliderMax}
            step={0.1}
            disabled={!currentTrack}
            onSeekStart={() => setSeeking(true)}
            onSeek={setTime}
            onSeekEnd={(value) => {
              setTime(value);
              setSeeking(false);
            }}
          />
          <span className='w-10 text-xs text-white/60 tabular-nums'>{formatTime(duration)}</span>
        </SliderFrame>
      </div>
      <VolumeBar
        className='absolute right-4 top-1/2 flex -translate-y-1/2 items-center gap-2 w-32'
        value={volume}
        onChange={setVolume}
      />
    </div>
  );
};
