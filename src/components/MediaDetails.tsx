import type { ComponentChildren } from 'preact';
import { Button } from '@/components/Button';
import { Artwork } from '@/components/MediaCard';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { useTrackCover } from '@/hooks/useTrackCover';

type Summary = { name: string; subtitle?: string | null; icon: IconName; path?: string | null };

export function MediaDetails({ item, metadata, next, onNext, children }: {
  item: Summary;
  metadata?: Array<{ icon: IconName; label: string }>;
  next?: Summary;
  onNext?: () => void;
  children?: ComponentChildren;
}) {
  const cover = useTrackCover(item.path ?? undefined);
  return (
    <aside className='collection-summary'>
      <Artwork className='collection-art' icon={item.icon} src={cover.coverSrc} onError={cover.clearCover} />
      <header>
        <h1>{item.name}</h1>
        {item.subtitle && <p>{item.subtitle}</p>}
      </header>
      {metadata && metadata.length > 0 && (
        <ul className='media-details-meta'>
          {metadata.map(({ icon, label }) => <li key={icon}><Icon name={icon} /><span>{label}</span></li>)}
        </ul>
      )}
      {next && <NextMedia item={next} onClick={onNext} />}
      {children && <div className='media-details-actions'>{children}</div>}
    </aside>
  );
}

function NextMedia({ item, onClick }: { item: Summary; onClick?: () => void }) {
  const cover = useTrackCover(item.path ?? undefined);
  const content = (
    <>
      <Artwork icon={item.icon} src={cover.coverSrc} onError={cover.clearCover} />
      <span className='track-label'>
        <span className='media-title'>{item.name}</span>
        {item.subtitle && <span className='track-artist'>{item.subtitle}</span>}
      </span>
    </>
  );
  return (
    <div className='media-details-next'>
      <p>Дальше</p>
      {onClick ? <Button variant='ghost' className='media-details-next-card' onClick={onClick}>{content}</Button>
        : <div className='media-details-next-card'>{content}</div>}
    </div>
  );
}
