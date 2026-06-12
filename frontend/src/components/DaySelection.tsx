import React from 'react';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';
import { Button } from './ui/button';

interface DaySelectionProps {
  days: string[];
  onDaySelect: (day: string) => void;
}

export function DaySelection({ days, onDaySelect }: DaySelectionProps) {
  return (
    <div className="space-y-4">
      <div>
        <p className="text-muted-foreground">Selecteer de dag waarvoor je de bootindeling wilt bekijken of bewerken.</p>
      </div>

      <div className="space-y-3">
        {days.map(day => (
          <Card key={day} className="cursor-pointer hover:shadow-md transition-shadow">
            <CardHeader className="pb-3">
              <CardTitle className="text-lg">{day}</CardTitle>
            </CardHeader>
            <CardContent className="pt-0">
              <Button 
                onClick={() => onDaySelect(day)}
                className="w-full"
              >
                {day}
              </Button>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}