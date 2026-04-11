# State Machine

# Write Path
{
  "Write DisPatch": 
    if has\_page() { 
    }else {
      reads.add(writeData.page, token);
      writes.add(token, writeData)
    }
}
