import { cn } from 'badlib';
import type { ComponentChildren, JSX, Ref } from 'preact';
import { Icon, type IconName } from '@/components/Icons/Icon';

export function Artwork({ icon, src, onError, className }: {
  icon: IconName;
  src?: string | null;
  onError?: () => void;
  className?: string;
}) {
  return (
    <span className={cn('kit-artwork', className)} aria-hidden='true'>
      {src ? <img src={src} alt='' decoding='async' onError={onError} /> : <Icon name={icon} />}
    </span>
  );
}

type MediaCardProps = Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, 'title'> & {
  title: string;
  subtitle?: string | null;
  metadata?: ComponentChildren;
  icon: IconName;
  coverSrc?: string | null;
  onCoverError?: () => void;
  kind?: 'track' | 'album' | 'artist' | 'playlist';
  buttonRef?: Ref<HTMLButtonElement>;
};

export function MediaCard({
  title, subtitle, metadata, icon, coverSrc, onCoverError, kind = 'track', buttonRef, className, ...props
}: MediaCardProps) {
  return (
    <button type='button' ref={buttonRef} className={cn('track-item', className)} data-kind={kind} {...props}>
      <Artwork className='track-cover' icon={icon} src={coverSrc} onError={onCoverError} />
      <span className='track-label'>
        <span className='media-title' title={title}>{title}</span>
        {subtitle && <span className='track-artist' title={subtitle}>{subtitle}</span>}
      </span>
      {metadata != null && <span className='track-duration'>{metadata}</span>}
    </button>
  );
}
