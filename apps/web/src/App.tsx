import { useState } from 'react'
import { LoginView } from './components/LoginView'
import { MainView } from './components/MainView'

export default function App() {
  const [isLoggedIn, setIsLoggedIn] = useState(false)
  return isLoggedIn 
    ? <MainView onLogout={() => setIsLoggedIn(false)} /> 
    : <LoginView onLogin={() => setIsLoggedIn(true)} />
}
