import { Button } from '@/components/Button';
import { Icon } from '@/components/Icons/Icon';

export function MediaFolderRow({ path, onRemove, disabled }: {
  path: string;
  onRemove: () => void;
  disabled: boolean;
}) {
  return (
    <div className='settings-folder'>
      <Icon name='Folder' size={32} />
      <p title={path}>{path}</p>
      <Button size='icon' className='settings-folder-remove' icon='Trash' disabled={disabled}
        aria-label={`Удалить папку ${path}`} onClick={onRemove} />
    </div>
  );
}
