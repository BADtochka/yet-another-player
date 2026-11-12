import { useState } from 'preact/hooks';
import { Link } from '@tanstack/react-router';
import { QueuePanel } from '@/components/QueuePanel';
import { SeekBar } from '@/components/SeekBar';
import { SliderFrame } from '@/components/Slider';
import { TrackPlaying } from '@/components/TrackPlaying';
import { VolumeBar } from '@/components/VolumeBar';
import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';
import { usePlayerStore } from '@/stores';

import { formatPlaybackTime } from '@/utils/formatPlaybackTime';

export const PlayerControls = () => {
  const currentTrack = usePlayerStore((state) => state.currentTrack);
  const playing = usePlayerStore((state) => state.playing);
  const time = usePlayerStore((state) => state.currentTime);
  const setPlaying = usePlayerStore((state) => state.setPlaying);
  const [drag, setDrag] = useState<{ path: string; position: number }>();
  const commitSeek = usePlayerStore((state) => state.commitSeek);
  const volume = usePlayerStore((state) => state.volume);
  const setVolume = usePlayerStore((state) => state.setVolume);
  const playNext = usePlayerStore((state) => state.playNext);
  const playPrevious = usePlayerStore((state) => state.playPrevious);
  const duration = usePlayerStore((state) => state.duration);
  const shuffle = usePlayerStore((state) => state.shuffle);
  const repeat = usePlayerStore((state) => state.repeat);
  const toggleShuffle = usePlayerStore((state) => state.toggleShuffle);
  const cycleRepeat = usePlayerStore((state) => state.cycleRepeat);
  const [queueOpen, setQueueOpen] = useState(false);
  const repeatLabel = { off: 'Повтор выключен', all: 'Повтор очереди', one: 'Повтор трека' }[repeat];
  const sliderMax = Math.max(duration, 0.1);
  const displayTime = drag && drag.path === currentTrack?.path ? drag.position : time;
  const sliderValue = Math.min(displayTime, sliderMax);

  return (
    <footer className='app-player' aria-label='Плеер'>
      <TrackPlaying track={currentTrack} />
      <div className='player-center'>
        <div className='player-buttons'>
          <Button variant='ghost' size='icon' icon='Shuffle' aria-label='Перемешать очередь' aria-pressed={shuffle} onClick={toggleShuffle} />
          <Button variant='ghost' size='icon' icon='Previous' aria-label='Предыдущий трек' onClick={playPrevious} />
          <Button variant='ghost' size='icon' className='player-play' aria-label={playing ? 'Пауза' : 'Воспроизвести'}
            disabled={!currentTrack} onClick={() => setPlaying(!playing)}>
            <Icon name={playing ? 'Pause' : 'Play'} size={50} />
          </Button>
          <Button variant='ghost' size='icon' icon='Next' aria-label='Следующий трек' onClick={playNext} />
          <Button variant='ghost' size='icon' icon='Repeat' className='relative' aria-label={repeatLabel} title={repeatLabel}
            aria-pressed={repeat !== 'off'} onClick={cycleRepeat}>
            {repeat === 'one' && <span className='repeat-one' aria-hidden='true'>1</span>}
          </Button>
        </div>
        <SliderFrame>
          <span className='player-time text-right'>{formatPlaybackTime(displayTime)}</span>
          <SeekBar
            label='Позиция воспроизведения'
            className='flex-1'
            value={sliderValue}
            min={0}
            max={sliderMax}
            step={0.1}
            disabled={!currentTrack || duration <= 0}
            onSeek={(position) => {
              if (currentTrack) setDrag({ path: currentTrack.path, position });
            }}
            onSeekEnd={(position) => {
              const path = drag?.path ?? currentTrack?.path;
              setDrag(undefined);
              if (path) commitSeek(path, position);
            }}
          />
          <span className='player-time'>{formatPlaybackTime(duration)}</span>
        </SliderFrame>
      </div>
      <div className='player-tools'>
        {currentTrack && <Link to='/lyrics' className='player-tool' aria-label='Текст песни' title='Текст песни'>
          <Icon name='Lyrics' />
        </Link>}
        <Button variant='ghost' size='icon' className='player-tool' icon='Queue' aria-label='Очередь' title='Очередь'
          aria-expanded={queueOpen} onClick={() => setQueueOpen(!queueOpen)} />
        <VolumeBar className='player-volume' value={volume} onChange={setVolume} />
        {queueOpen && <QueuePanel close={() => setQueueOpen(false)} />}
      </div>
    </footer>
  );
};
