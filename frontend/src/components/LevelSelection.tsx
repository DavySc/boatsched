import React from 'react';
import { Card, CardContent, CardHeader, CardTitle } from './ui/card';
import { Button } from './ui/button';

interface LevelSelectionProps {
  onLevelSelect: (level: number) => void;
}

export function LevelSelection({ onLevelSelect }: LevelSelectionProps) {
  const levels = [
    { number: 1, name: "Ketelbinkie", description: "Kennismaking met het zeilen" },
    { number: 2, name: "Bemanning", description: "Perfectioneren van het Ketelbinkie niveau en kennismaking met eerste manoeuvres" },
    { number: 3, name: "A", description: "Perfectioneren van het Bemanning niveau en kennismaking met moeilijkere manoeuvres" },
    { number: 4, name: "B", description: "Perfectioneren van het zeilen, manoeuvres en organisatie" }
  ];

  return (
    <div className="space-y-4">
      <div>
        <p className="text-muted-foreground">Kies het niveau van de kamper waarvoor je voortgang wilt bijhouden.</p>
      </div>

      <div className="space-y-3">
        {levels.map(level => (
          <Card key={level.number} className="cursor-pointer hover:shadow-md transition-shadow">
            <CardHeader className="pb-3">
              <CardTitle className="text-lg">{level.name}</CardTitle>
            </CardHeader>
            <CardContent className="space-y-3 pt-0">
              <p className="text-sm text-muted-foreground">{level.description}</p>
              <Button 
                onClick={() => onLevelSelect(level.number)}
                className="w-full"
              >
                {level.name}
              </Button>
            </CardContent>
          </Card>
        ))}
      </div>
    </div>
  );
}