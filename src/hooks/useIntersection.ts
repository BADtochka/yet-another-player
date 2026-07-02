import { type RefObject } from 'preact';
import { useEffect, useState } from 'preact/hooks';

export const useIntersection = (ref: RefObject<HTMLElement>, rootMargin = '200px') => {
  const [isIntersecting, setIsIntersecting] = useState(false);

  useEffect(() => {
    const node = ref.current;

    if (!node || isIntersecting) {
      return;
    }

    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry?.isIntersecting) {
          setIsIntersecting(true);
          observer.disconnect();
        }
      },
      { rootMargin },
    );

    observer.observe(node);

    return () => {
      observer.disconnect();
    };
  }, [ref, isIntersecting, rootMargin]);

  return isIntersecting;
};
