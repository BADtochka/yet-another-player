import { createFileRoute } from '@tanstack/react-router';

const Index = () => {
  return <h1>Альбомы</h1>;
};

export const Route = createFileRoute('/')({
  component: Index,
});
