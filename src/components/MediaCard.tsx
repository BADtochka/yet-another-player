import { cn } from 'badlib';
import { useState } from 'preact/hooks';
import type { ComponentChildren, JSX, Ref } from 'preact';
import { Icon, type IconName } from '@/components/Icons/Icon';
import { Spinner } from '@/components/LoadingIndicator';

function largerAppleArtwork(src: string) {
  return src.replace(/^(https:\/\/[^/]+\.mzstatic\.com\/[^?#]*?)(?:100x100(?:-[0-9]+)?(?:bb)?)(\.(?:jpe?g|png)(?:[?#].*)?)$/iu, (_, prefix: string, suffix: string) => `${prefix}600x600bb${suffix}`);
}

export function Artwork({ icon, src, onError, className, children, pending }: {
  icon: IconName;
  src?: string | null;
  onError?: () => void;
  className?: string;
  children?: ComponentChildren;
  pending?: boolean;
}) {
  const [failedLarge, setFailedLarge] = useState<string>();
  const [loaded, setLoaded] = useState<string>();
  const large = src && src !== failedLarge ? largerAppleArtwork(src) : src;
  const waiting = pending || (pending !== undefined && !!large && loaded !== large);
  return (
    <span className={cn('kit-artwork', className)} aria-hidden='true'>
      {src ? <img src={large ?? undefined} alt='' decoding='async'
        onLoad={pending === undefined ? undefined : () => setLoaded(large ?? undefined)}
        onError={() => large !== src ? setFailedLarge(src) : onError?.()} /> : !waiting && <Icon name={icon} />}
      {waiting && <span className='kit-artwork-progress'><Spinner /></span>}
      {children}
    </span>
  );
}

type MediaCardProps = Omit<JSX.ButtonHTMLAttributes<HTMLButtonElement>, 'title'> & {
  title: string;
  subtitle?: string | null;
  metadata?: ComponentChildren;
  icon: IconName;
  coverSrc?: string | null;
  coverPending?: boolean;
  onCoverError?: () => void;
  kind?: 'track' | 'album' | 'artist' | 'playlist';
  buttonRef?: Ref<HTMLButtonElement>;
  titleLink?: ComponentChildren;
  subtitleLinks?: ComponentChildren;
  action?: ComponentChildren;
  playable?: boolean;
  'data-track-path'?: string;
  'data-external-video'?: string;
  'data-local-video'?: boolean;
};

export function MediaCard({
  title, titleLink, subtitle, subtitleLinks, action, metadata, icon, coverSrc, coverPending, onCoverError, kind = 'track', buttonRef, className, playable,
  'data-track-path': path, 'data-external-video': video, 'data-local-video': hasLocalVideo, ...props
}: MediaCardProps) {
  const cardProps = { 'data-kind': kind, 'data-track-path': path, 'data-external-video': video, 'data-local-video': hasLocalVideo };
  const content = (
    <>
      <Artwork className='track-cover' icon={icon} src={coverSrc} pending={coverPending} onError={onCoverError}>
        {playable && !props.disabled && <span className='track-cover-play'><Icon name='PlayV2' /></span>}
      </Artwork>
      <span className='track-label'>
        <span className='media-title' title={title}>{titleLink ?? title}</span>
        {(subtitle || subtitleLinks) && <span className='track-artist' title={subtitle ?? undefined}>{subtitleLinks ?? subtitle}</span>}
      </span>
      {metadata != null && <span className='track-duration'>{metadata}</span>}
    </>
  );
  if (action != null) {
    return <div className={cn('track-item', 'track-item-with-links', 'track-item-with-action', className)} {...cardProps}>
      {content}{action}
    </div>;
  }
  if (titleLink || subtitleLinks) {
    return <div className={cn('track-item', 'track-item-with-links', className)} {...cardProps}>
      <button type='button' ref={buttonRef} className='track-item-play'
        aria-label={`${kind === 'track' ? 'Воспроизвести' : 'Открыть'} «${title}»`} {...props} />
      {content}
    </div>;
  }
  return <button type='button' ref={buttonRef} className={cn('track-item', className)} {...cardProps} {...props}>{content}</button>;
}
