import { useAuth } from '../context/AuthContext';

export function useAuthStatus() {
  return useAuth();
}
