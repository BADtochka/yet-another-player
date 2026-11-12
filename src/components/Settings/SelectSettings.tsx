import { Select, type SelectProps } from '@/components/Select';

export function SelectSettings<T extends string>(props: SelectProps<T>) {
  return <div className='settings-row'>
    <p>{props.label}</p>
    <Select {...props} />
  </div>;
}
