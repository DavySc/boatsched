import React, { useState } from 'react';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';
import { Input } from './ui/input';
import { Button } from './ui/button';
import { Label } from './ui/label';
import { Alert, AlertDescription } from './ui/alert';
import { toast } from 'sonner';
import pzkLogo from '../assets/pzk-logo.png';

interface LoginProps {
  onLogin: () => void;
}

export function Login({ onLogin }: LoginProps) {
  const [name, setName] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState('');
  const [isLoading, setIsLoading] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsLoading(true);
    setError('');
    try {
      const resp = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: name.trim(), password }),
        credentials: 'same-origin',
      });
      if (resp.ok) {
        toast.success('Welkom bij de Zeilkamp App!');
        onLogin();
      } else {
        const data = await resp.json().catch(() => ({}));
        setError(data.error || 'Onjuiste naam of wachtwoord. Probeer opnieuw.');
        setPassword('');
      }
    } catch {
      setError('Kan geen verbinding maken met de server.');
    } finally {
      setIsLoading(false);
    }
  };

  return (
    <div className="min-h-screen bg-gradient-to-br from-[#FFB8B8] to-[#FFB8B8]/70 flex items-center justify-center p-4">
      <div className="w-full max-w-md space-y-6">
        <div className="text-center space-y-4">
          <div className="flex justify-center">
            <div className="w-20 h-20 rounded-full flex items-center justify-center p-2" style={{ backgroundColor: '#E60000' }}>
              <img src={pzkLogo} alt="PZK Logo" className="w-full h-full object-contain" />
            </div>
          </div>
          <div>
            <h1 className="text-3xl" style={{ color: '#E60000' }}>Zeilkamp App</h1>
            <p className="mt-2" style={{ color: '#E60000' }}>Evaluatiekaarten voor zeilinstructie</p>
          </div>
        </div>

        <Card className="shadow-lg border-0">
          <CardHeader className="space-y-2">
            <CardTitle className="text-xl text-center">Inloggen</CardTitle>
            <p className="text-sm text-muted-foreground text-center">
              Voer uw naam en wachtwoord in om toegang te krijgen
            </p>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSubmit} className="space-y-4">
              <div className="space-y-2">
                <Label htmlFor="name">Naam</Label>
                <Input
                  id="name"
                  type="text"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Uw naam"
                  required
                  disabled={isLoading}
                  className="text-base"
                  autoComplete="username"
                />
              </div>
              <div className="space-y-2">
                <Label htmlFor="password">Wachtwoord</Label>
                <Input
                  id="password"
                  type="password"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="Voer wachtwoord in"
                  required
                  disabled={isLoading}
                  className="text-base"
                  autoComplete="current-password"
                />
              </div>

              {error && (
                <Alert variant="destructive">
                  <AlertDescription>{error}</AlertDescription>
                </Alert>
              )}

              <Button
                type="submit"
                className="w-full text-white hover:opacity-90"
                style={{ backgroundColor: '#E60000' }}
                disabled={isLoading || !name.trim() || !password.trim()}
              >
                {isLoading ? 'Inloggen...' : 'Toegang krijgen'}
              </Button>
            </form>
          </CardContent>
        </Card>

        <div className="text-center text-sm" style={{ color: '#E60000' }}>
          <p>Voor schippers en zeilinstructeurs</p>
        </div>
      </div>
    </div>
  );
}
