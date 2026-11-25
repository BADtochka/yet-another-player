import { useEffect, useRef } from 'preact/hooks';
import { TrackArtistLinks } from '@/components/TrackLinks';

export function ScrollingTrackArtists({ artists }: { artists: string[] }) {
  const line = useRef<HTMLSpanElement>(null);
  const text = useRef<HTMLSpanElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const animation = useRef<Animation>();

  useEffect(() => () => animation.current?.cancel(), []);

  return (
    <>
      <span ref={line} className='track-artist scrolling-track-artists'
        onPointerEnter={() => {
          const distance = (text.current?.offsetWidth ?? 0) - (line.current?.clientWidth ?? 0);
          if (distance <= 0 || !text.current || matchMedia('(prefers-reduced-motion: reduce)').matches) return;
          animation.current = text.current.animate(
            [{ transform: 'translateX(0)' }, { transform: `translateX(-${distance}px)` }],
            { delay: 350, duration: Math.max(800, distance * 25), fill: 'forwards' },
          );
        }}
        onPointerLeave={() => animation.current?.cancel()}
        onFocusIn={() => animation.current?.cancel()}
        onContextMenu={(event) => {
          if (artists.length < 2 || !list.current) return;
          event.preventDefault();
          const panel = list.current;
          if (!panel.matches(':popover-open')) panel.showPopover();
          const rect = event.currentTarget.getBoundingClientRect();
          panel.style.left = `${Math.max(8, Math.min(event.clientX || rect.left, innerWidth - panel.offsetWidth - 8))}px`;
          panel.style.top = `${Math.max(8, Math.min(event.clientY || rect.bottom, innerHeight - panel.offsetHeight - 8))}px`;
        }}>
        <span ref={text} className='scrolling-track-artists-text'><TrackArtistLinks artists={artists} /></span>
      </span>
      {artists.length > 1 && (
        <div ref={list} className='track-context-menu track-artist-popover' popover='auto' aria-label='Исполнители трека'>
          <ul>
            {artists.map((artist, index) => (
              <li key={index} onClick={() => list.current?.hidePopover()}><TrackArtistLinks artists={[artist]} /></li>
            ))}
          </ul>
        </div>
      )}
    </>
  );
}
